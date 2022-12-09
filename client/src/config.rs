use std::path::PathBuf;

use anyhow::{anyhow, Error};
use place_rs_shared::{program_path, safe_get_from_terminal};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub https: HttpsOrPort,
    pub url: String,
}

impl Config {
    pub fn new() -> Result<Config, Error> {
        let cfg = Self::try_load()?;
        let mut changed = false;
        let cfg = Config {
            https: cfg.https.unwrap_or_else(|| {
                // ask for https y/n
                let mut https = String::new();
                loop {
                    println!("Use https? (y/n):");
                    std::io::stdin().read_line(&mut https).unwrap();
                    match https.trim() {
                        "y" => break HttpsOrPort::Https,
                        "n" => {
                            break HttpsOrPort::Port(
                                // ask for port, if number, set it, if n set None
                                loop {
                                    let mut port = String::new();
                                    println!("Port (number or n):");
                                    std::io::stdin().read_line(&mut port).unwrap();
                                    match port.trim() {
                                        "n" => break None,
                                        _ => match port.trim().parse::<u16>() {
                                            Ok(port) => break Some(port),
                                            Err(_) => println!("Invalid port"),
                                        },
                                    }
                                },
                            );
                        }
                        _ => println!("Invalid input"),
                    }
                }
            }),
            url: cfg.url.unwrap_or_else(|| {
                changed = true;
                safe_get_from_terminal("url")
            }),
        };
        if changed {
            Self::save(&cfg)?;
        }
        Ok(cfg)
    }
    pub fn get_http_url(&self) -> String {
        // if https is Https, return https://url
        match self.https {
            HttpsOrPort::Https => format!("https://{}", self.url),
            HttpsOrPort::Port(port) => format!(
                "http://{}{}",
                self.url,
                match port {
                    Some(port) => format!(":{}", port),
                    None => "".to_string(),
                }
            ),
        }
    }
    pub fn get_ws_url(&self) -> String {
        // if https is Https, return wss://url
        match self.https {
            HttpsOrPort::Https => format!("wss://{}", self.url),
            HttpsOrPort::Port(port) => format!(
                "ws://{}{}",
                self.url,
                match port {
                    Some(port) => format!(":{}", port),
                    None => "".to_string(),
                }
            ),
        }
    }
    fn try_load() -> Result<PossibleConfig, Error> {
        let path = Self::file_path()?;
        if path.exists() {
            let file = std::fs::File::open(path)?;
            let reader = std::io::BufReader::new(file);
            let r = serde_json::from_reader(reader)?;
            Ok(r)
        } else {
            Ok(PossibleConfig::default())
        }
    }
    pub fn save(cfg: &Config) -> Result<PathBuf, Error> {
        let path = Self::file_path()?;
        std::fs::create_dir_all(path.parent().ok_or_else(|| anyhow!("No parent"))?)?;
        let file = std::fs::File::create(path.clone())?;
        let writer = std::io::BufWriter::new(file);
        serde_json::to_writer_pretty(writer, &cfg)?;
        Ok(path)
    }
    fn file_path() -> Result<std::path::PathBuf, Error> {
        Ok(program_path()?.join("client_config.json"))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HttpsOrPort {
    Https,
    Port(Option<u16>),
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct PossibleConfig {
    pub https: Option<HttpsOrPort>,
    pub url: Option<String>,
}
