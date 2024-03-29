use pico_common::Driver;
use pico_download::{cache_resolution, download_drivers_to_cache};

#[test]
fn load_all_drivers() {
    let drivers = [
        Driver::PS2000,
        Driver::PS2000A,
        Driver::PS3000A,
        Driver::PS4000,
        Driver::PS4000A,
        Driver::PS5000A,
        Driver::PS6000,
    ];

    let cache_resolution = cache_resolution();

    drivers.into_iter().for_each(|d| {
        let mut loaded = cache_resolution.try_load(d);

        if loaded.is_err() {
            download_drivers_to_cache(&[d]).unwrap();
            loaded = cache_resolution.try_load(d);
        }

        assert!(loaded.is_ok());
        let loaded = loaded.unwrap();

        assert!(loaded.get_version().is_ok());
    });
}
