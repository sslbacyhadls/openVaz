pub mod cabin;
pub mod engine;
pub mod fb;
pub mod firmware;
pub mod hardware;
pub mod layout;
// The shared port contract is formatted separately by its owners.
#[rustfmt::skip]
pub mod link;
pub mod model;
pub mod paint;
pub mod vehicle;
pub mod watchdog;

#[cfg(feature = "window")]
pub mod car_window;
#[cfg(feature = "window")]
pub mod window;
