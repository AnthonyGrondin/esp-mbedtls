#![no_std]

pub mod c_types;

#[cfg_attr(feature = "esp32", path = "include/esp32.rs")]
#[cfg_attr(feature = "esp32c3", path = "include/esp32c3.rs")]
#[cfg_attr(feature = "esp32s2", path = "include/esp32s2.rs")]
#[cfg_attr(feature = "esp32s3", path = "include/esp32s3.rs")]
pub mod bindings;
