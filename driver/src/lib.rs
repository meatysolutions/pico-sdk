//! Common, safe wrappers for Pico Technology oscilloscope drivers.
//!
//! This is a sub crate that you probably don't want to use directly. Try the top level
//! [`pico-sdk`](https://crates.io/crates/pico-sdk) crate which exposes everything from here.
//!
//! Each Pico Technology oscilloscope relies on a native driver for communication and this driver will
//! vary depending on the device product range. Each of these drivers has an interface which differs by
//! either a few function arguments or a vastly differing API.
//!
//! `PS2000Driver`, `PS2000ADriver`, `PS3000ADriver`, `PS4000Driver`,
//! `PS4000ADriver`, `PS5000ADriver`, `PS6000Driver` and `PS6000ADriver` wrap
//! their corresponding loaders and expose a safe, common API by implementing
//! the `PicoDriver` trait. These can be constructed with a `Resolution` which tells the wrapper where
//! to resolve the dynamic library from.
//!
//! # Examples
//! Using the raw safe bindings to open and configure the first available device:
//! ```no_run
//! # fn run() -> Result<(),Box<dyn std::error::Error>> {
//! use pico_common::{ChannelConfig, Driver, PicoChannel, PicoCoupling, PicoInfo, PicoRange};
//! use pico_driver::LibraryResolution;
//!
//! // Load the ps2000 driver library with the default resolution
//! let driver = LibraryResolution::Default.try_load(Driver::PS2000)?;
//! // Load the ps4000a driver library from the applications root directory
//! let driver = LibraryResolution::AppRoot.try_load(Driver::PS4000A)?;
//!
//! // Open the first device
//! let handle = driver.open_unit(None)?;
//! let variant = driver.get_unit_info(handle, PicoInfo::VARIANT_INFO)?;
//!
//! let ch_config = ChannelConfig {
//!     coupling: PicoCoupling::DC,
//!     range: PicoRange::X1_PROBE_2V,
//!     offset: 0.0
//! };
//!
//! driver.enable_channel(handle, PicoChannel::A, &ch_config)?;
//! # Ok(())
//! # }
//! ```

#![allow(clippy::upper_case_acronyms)]

use parking_lot::RwLock;
use pico_common::{
    ChannelConfig, Driver, FromPicoStr, PicoChannel, PicoError, PicoInfo, PicoRange, PicoResult,
    SampleConfig,
};
pub use resolution::LibraryResolution;
use std::{fmt, sync::Arc};
use thiserror::Error;
use version_compare::Version;

mod dependencies;
pub mod kernel_driver;
pub mod ps2000;
pub mod ps2000a;
pub mod ps3000a;
pub mod ps4000;
pub mod ps4000a;
pub mod ps5000a;
pub mod ps6000;
pub mod ps6000a;
mod resolution;
mod trampoline;

/// Covers the various errors encountered when attempting to load drivers
#[derive(Error, Debug)]
pub enum DriverLoadError {
    #[error("Pico driver error: {0}")]
    DriverError(#[from] PicoError),

    #[error("Library load error: {0}")]
    LibloadingError(#[from] libloading::Error),

    #[error("Invalid Driver Version: Requires >= {required}, Found: {found}")]
    VersionError { found: String, required: String },
}

/// Serial and variant pairs returned by driver enumeration
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug)]
pub struct EnumerationResult {
    pub variant: String,
    pub serial: String,
}

/// Common trait implemented for every driver
pub trait PicoDriver: fmt::Debug + Send + Sync {
    /// Gets the underlying driver type
    fn get_driver(&self) -> Driver;
    /// Gets the driver version string
    fn get_version(&self) -> PicoResult<String>;
    /// Gets the path to the loaded driver
    fn get_path(&self) -> PicoResult<Option<String>>;
    /// Returns a list of discovered serial numbers
    fn enumerate_units(&self) -> PicoResult<Vec<EnumerationResult>>;
    /// Opens a device, optionally with a specific serial number
    fn open_unit(&self, serial: Option<&str>) -> PicoResult<i16>;
    /// Ping a unit to see if it's still connected
    fn ping_unit(&self, handle: i16) -> PicoResult<()>;
    /// Get the maximum expected ADC value. This is required to scale to volts
    fn maximum_value(&self, handle: i16) -> PicoResult<i16>;
    /// Close the specified unit
    fn close(&self, handle: i16) -> PicoResult<()>;
    /// Get one of the unit info strings
    fn get_unit_info(&self, handle: i16, info_type: PicoInfo) -> PicoResult<String>;
    /// Get valid ranges for the specified channel
    fn get_channel_ranges(&self, handle: i16, channel: PicoChannel) -> PicoResult<Vec<PicoRange>>;
    /// Set up a channel with the supplied config
    fn enable_channel(
        &self,
        handle: i16,
        channel: PicoChannel,
        config: &ChannelConfig,
    ) -> PicoResult<()>;
    /// Disable a channel
    fn disable_channel(&self, handle: i16, channel: PicoChannel) -> PicoResult<()>;
    /// Give the driver a buffer to write data into
    fn set_data_buffer(
        &self,
        handle: i16,
        channel: PicoChannel,
        buffer: Arc<RwLock<Vec<i16>>>,
        buffer_len: usize,
    ) -> PicoResult<()>;
    /// Starts the device streaming
    fn start_streaming(
        &self,
        handle: i16,
        sample_config: &SampleConfig,
        enabled_channels: u8,
    ) -> PicoResult<SampleConfig>;
    /// Gets the latest streaming values
    fn get_latest_streaming_values<'a>(
        &self,
        handle: i16,
        channels: &[PicoChannel],
        callback: Box<dyn FnMut(usize, usize) + 'a>,
    ) -> PicoResult<()>;
    /// Stops the device streaming
    fn stop(&self, handle: i16) -> PicoResult<()>;
    /// Check that the driver meets the minimum version tested with these wrappers
    fn check_version(&self) -> Result<(), DriverLoadError> {
        let loaded_str = &self.get_version()?;
        let loaded_version = Version::from(loaded_str);

        let required_str = get_min_required_version(self.get_driver());
        let required_version = Version::from(required_str);

        if loaded_version < required_version {
            Err(DriverLoadError::VersionError {
                found: loaded_str.to_string(),
                required: required_str.to_string(),
            })
        } else {
            Ok(())
        }
    }
}

pub type ArcDriver = Arc<dyn PicoDriver>;

pub(crate) fn get_version_string(input: &str) -> String {
    input
        .split(|s| s == ' ' || s == ',')
        .last()
        .expect("Invalid version string")
        .to_string()
}

pub(crate) fn parse_enum_result(buffer: &[i8], len: usize) -> Vec<EnumerationResult> {
    let serials_list = buffer.from_pico_i8_string(len);

    serials_list
        .split(',')
        .map(String::from)
        .map(|device| {
            let parts = device.split('[').collect::<Vec<_>>();

            EnumerationResult {
                serial: parts[0].to_string(),
                variant: parts[1].trim_end_matches(']').to_string(),
            }
        })
        .collect()
}

/// Gets the minimum supported driver version
fn get_min_required_version(driver: Driver) -> &'static str {
    match driver {
        Driver::PS2000 => "3.0.30.1878",
        Driver::PS2000A
        | Driver::PS3000A
        | Driver::PS4000
        | Driver::PS4000A
        | Driver::PS5000A
        | Driver::PS6000 => "2.1.30.1878",
        Driver::PS6000A => "1.0.54.2438",
        _ => panic!(
            "We don't know the minimum required version for the {:?} driver!",
            driver,
        ),
    }
}
