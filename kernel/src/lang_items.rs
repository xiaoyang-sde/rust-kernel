//! The `lang_items` module contains Rust lang items.
//! Rust lang items are functionalities that isn't hard-coded into the language,
//! but is implemented in libraries, with a special marker to tell the compiler it exists.
//! Since the kernel doesn't depend on the `std` crate, it has to implement some
//! lang items, such as the `panic_handler`.

use crate::{println, sbi::shutdown};
use core::panic::PanicInfo;

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    if let Some(location) = info.location() {
        println!(
            "[kernel] panic at {}:{}: {}",
            location.file(),
            location.line(),
            info.message().unwrap()
        );
    } else {
        println!("[kernel] panic: {}", info.message().unwrap());
    }
    shutdown();
}
