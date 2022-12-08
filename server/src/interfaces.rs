#![allow(dead_code)]

use std::io::{Read, Write};

use anyhow::{anyhow, Error};
use place_rs_shared::{PixelWithLocation, Place, PlaceInterface, XY};
use serde::{Deserialize, Serialize};

use crate::CONFIG;

// json interface
pub struct JsonInterface {
    path: std::path::PathBuf,
}

impl JsonInterface {
    pub fn new(path: std::path::PathBuf) -> JsonInterface {
        JsonInterface { path }
    }
}

#[async_trait::async_trait]
impl PlaceInterface for JsonInterface {
    async fn load(&self) -> Result<place_rs_shared::Place, Error> {
        let file = std::fs::File::open(&self.path);
        if let Ok(file) = file {
            let reader = std::io::BufReader::new(file);
            let r: Place = serde_json::from_reader(reader)?;
            if XY::from_nested_vec(&r.data).unwrap() != CONFIG.size {
                println!("Size mismatch, if you want to change the size, delete or rename `{:?}` and restart the server.", &self.path)
            }
            Ok(r)
        } else {
            let place = Place::new(CONFIG.size);
            self.save_all(&place).await?;
            Ok(place)
        }
    }
    async fn save_all(&self, place: &place_rs_shared::Place) -> Result<(), Error> {
        std::fs::create_dir_all(self.path.parent().ok_or_else(|| anyhow!("No parent"))?)?;
        let file = std::fs::File::create(&self.path)?;
        let writer = std::io::BufWriter::new(file);
        serde_json::to_writer_pretty(writer, &place)?;
        Ok(())
    }
    async fn save_pixel(&self, _pixel: &PixelWithLocation) -> Result<(), Error> {
        // let mut place = self.load().await?;
        // place.data[pixel.location.y as usize][pixel.location.x as usize] = pixel.clone();
        // self.save(&place).await
        Ok(())
    }
    async fn save_user(&self, _user: &place_rs_shared::User) -> Result<(), Error> {
        // let mut place = self.load().await?;
        // place.users.insert(user.id, user.clone());
        // self.save(&place).await
        Ok(())
    }
}

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
pub enum Interface {
    #[default]
    Json,
    Gzip,
    Postgres(PostgresConfig),
}

// interface using the crate flate2 to gzip the data or ungzip it
pub struct GzipInterface {
    path: std::path::PathBuf,
}

impl GzipInterface {
    pub fn new(path: std::path::PathBuf) -> GzipInterface {
        GzipInterface { path }
    }
}

#[async_trait::async_trait]
impl PlaceInterface for GzipInterface {
    async fn load(&self) -> Result<place_rs_shared::Place, Error> {
        let file = std::fs::File::open(&self.path);
        if let Ok(file) = file {
            // getting a [u8] from the file, then using Place::gun_unzip() to get the Place
            let mut file = std::io::BufReader::new(file);
            let mut buffer = Vec::new();
            file.read_to_end(&mut buffer)?;
            let r = Place::gun_unzip(&buffer)?;
            if XY::from_nested_vec(&r.data).unwrap() != CONFIG.size {
                println!("Size mismatch, if you want to change the size, delete or rename `{:?}` and restart the server.", &self.path)
            }
            Ok(r)
        } else {
            let place = Place::new(CONFIG.size);
            self.save_all(&place).await?;
            Ok(place)
        }
    }
    async fn save_all(&self, place: &place_rs_shared::Place) -> Result<(), Error> {
        std::fs::create_dir_all(self.path.parent().ok_or_else(|| anyhow!("No parent"))?)?;
        // using place.gun_zip() to get the zipped data, then writing it to the file
        let mut file = std::fs::File::create(&self.path)?;
        let data = place.gun_zip()?;
        file.write_all(&data)?;
        Ok(())
    }
    async fn save_pixel(&self, _pixel: &PixelWithLocation) -> Result<(), Error> {
        // let mut place = self.load().await?;
        // place.data[pixel.location.y as usize][pixel.location.x as usize] = pixel.clone();
        // self.save(&place).await
        Ok(())
    }
    async fn save_user(&self, _user: &place_rs_shared::User) -> Result<(), Error> {
        // let mut place = self.load().await?;
        // place.users.insert(user.id, user.clone());
        // self.save(&place).await
        Ok(())
    }
}

#[derive(Debug)]
pub struct PostgresInterface {
    pool: sqlx::PgPool,
}

impl PostgresInterface {
    pub async fn new(config: PostgresConfig) -> Result<PostgresInterface, Error> {
        let pool = sqlx::PgPool::connect(&format!("postgres://{}:{}@{}:{}/{}", config.user, config.password, config.host, config.port, config.database)).await?;
        Ok(PostgresInterface { pool })
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PostgresConfig {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: String,
    pub database: String,
}
