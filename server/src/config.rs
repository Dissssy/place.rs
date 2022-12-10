use std::path::PathBuf;

use anyhow::{anyhow, Error};
use place_rs_shared::{program_path, safe_get_from_terminal, XY};
use serde::{Deserialize, Serialize};

use crate::interfaces::{Interface, PostgresConfig};

#[derive(Debug, Serialize)]
pub struct Config {
    pub port: u16,
    pub host: String,
    pub size: XY,
    pub interface: Interface,
    pub times: Times,
    pub timeouts: Timeouts,
    pub max_missed_heartbeats: u32,
}

impl Config {
    pub fn new() -> Result<Config, Error> {
        let cfg = Self::try_load()?;
        let mut changed = false;
        let cfg = Config {
            port: cfg.port.unwrap_or_else(|| {
                changed = true;
                safe_get_from_terminal("port")
            }),
            host: cfg.host.unwrap_or_else(|| {
                changed = true;
                safe_get_from_terminal("host")
            }),
            size: cfg.size.unwrap_or_else(|| {
                changed = true;
                XY {
                    x: safe_get_from_terminal("canvas width"),
                    y: safe_get_from_terminal("canvas height"),
                }
            }),
            interface: cfg.interface.unwrap_or_else(|| {
                changed = true;
                let mut interface = String::new();
                loop {
                    println!("Interface (json, gzip, postgres):");
                    std::io::stdin().read_line(&mut interface).unwrap();
                    match interface.trim() {
                        "json" => break Interface::Json,
                        "gzip" => break Interface::Gzip,
                        "postgres" => {
                            break Interface::Postgres(PostgresConfig {
                                host: safe_get_from_terminal("postgres host"),
                                port: safe_get_from_terminal("posgtres port"),
                                user: safe_get_from_terminal("postgres user"),
                                password: safe_get_from_terminal("postgres password"),
                                database: safe_get_from_terminal("postgres database"),
                            })
                        }
                        _ => println!("Invalid interface"),
                    }
                }
            }),
            times: cfg.times.unwrap_or_else(|| {
                changed = true;
                Times {
                    ws_msg_interval: safe_get_from_terminal("websocket message interval (ms)"),
                    ws_hb_interval: safe_get_from_terminal("websocket heartbeat interval (ms)"),
                }
            }),
            timeouts: cfg.timeouts.unwrap_or_else(|| {
                changed = true;
                Timeouts {
                    paint: safe_get_from_terminal("paint timeout (seconds)"),
                    username: safe_get_from_terminal("username timeout (seconds)"),
                    chat: safe_get_from_terminal("chat timeout (ms)"),
                }
            }),
            max_missed_heartbeats: cfg.max_missed_heartbeats.unwrap_or_else(|| {
                changed = true;
                safe_get_from_terminal("max missed heartbeats")
            }),
        };
        if changed {
            println!("Config saved to {:?}", Self::save(&cfg)?);
        }
        Ok(cfg)
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
        Ok(program_path()?.join("server_config.json"))
    }
}

#[derive(Deserialize, Default)]
struct PossibleConfig {
    port: Option<u16>,
    host: Option<String>,
    size: Option<XY>,
    interface: Option<Interface>,
    times: Option<Times>,
    timeouts: Option<Timeouts>,
    max_missed_heartbeats: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy)]
pub struct Times {
    pub ws_msg_interval: u64,
    pub ws_hb_interval: u64,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, Default)]
pub struct Timeouts {
    pub paint: u64,
    pub username: u64,
    pub chat: u64,
}
