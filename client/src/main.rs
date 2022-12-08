use lazy_static::lazy_static;
mod config;
use config::Config;

lazy_static! {
    pub static ref CONFIG: Config = Config::new().unwrap();
}

#[tokio::main]
async fn main() {}
