mod exchange;

pub use exchange::{AssetExchange, BithumbAssetClient};

use std::sync::Once;

static INIT: Once = Once::new();

/// Initialize the library (loads environment variables from .env file)
/// This is automatically called when the library is loaded
fn init() {
    INIT.call_once(|| {
        dotenv::dotenv().ok();
    });
}

// Automatically initialize when the library is loaded
#[ctor::ctor]
fn setup() {
    init();
}
