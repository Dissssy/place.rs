#![allow(dead_code)]
use anyhow::{anyhow, Error};
use image::GenericImage;
use messages::ToClientMsg;
use serde::{Deserialize, Serialize};
use sha2::Digest;
use sha2::Sha256;
use std::io::BufWriter;
use std::path::PathBuf;
use std::{
    collections::HashMap,
    fmt::Formatter,
    io::{Read, Write},
};
use tokio::sync::mpsc::UnboundedSender;
pub mod messages;

#[derive(Deserialize, Default, Debug, Serialize, Clone, PartialEq, Eq)]
pub struct Place {
    pub data: Vec<Vec<PixelWithLocation>>,
    pub users: HashMap<String, User>,
}

impl Place {
    pub fn new(size: XY) -> Place {
        Place {
            data: vec![vec![PixelWithLocation::default(); size.x as usize]; size.y as usize],
            users: HashMap::new(),
        }
    }
    pub async fn gun_zip(&self) -> Result<Vec<u8>, Error> {
        // spawn in seperate thread to avoid blocking the async runtime
        let (tx, rx) = tokio::sync::oneshot::channel();
        let p = self.clone();
        std::thread::spawn(move || {
            let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
            encoder.write_all(&serde_json::to_vec(&p).unwrap()).unwrap();
            tx.send(encoder.finish().unwrap()).unwrap();
        });
        Ok(rx.await.unwrap())
    }
    pub async fn gun_unzip(data: Vec<u8>) -> Result<Place, Error> {
        // spawn in seperate thread to avoid blocking the async runtime
        let (tx, rx) = tokio::sync::oneshot::channel();
        std::thread::spawn(move || {
            let mut decoder = flate2::read::GzDecoder::new(&data[..]);
            let mut buffer = Vec::new();
            decoder.read_to_end(&mut buffer).unwrap();
            tx.send(serde_json::from_slice(&buffer).unwrap()).unwrap();
        });
        Ok(rx.await.unwrap())
    }
    pub async fn get_img(&self, path: PathBuf) -> Result<(), Error> {
        // create a new DynamicImage with the same dimensions as the image
        let mut img = image::DynamicImage::new_rgba8(self.data[0].len() as u32, self.data.len() as u32);
        for (y, row) in self.data.iter().enumerate() {
            for (x, pixel) in row.iter().enumerate() {
                let color = match &pixel.pixel {
                    MaybePixel::Pixel(p) => p.color,
                    MaybePixel::None => Color::default(),
                };
                img.put_pixel(x as u32, y as u32, image::Rgba([color.r, color.g, color.b, 255]));
            }
        }
        // write the contents of this image to memory
        let mut buffer = BufWriter::new(std::fs::File::create(path)?);
        img.write_to(&mut buffer, image::ImageOutputFormat::Png)?;
        Ok(())
    }
    pub fn set_pixel(&mut self, pixel: PixelWithLocation) -> Result<(), Error> {
        let PixelWithLocation { pixel, location } = pixel;
        if location.x >= self.data[0].len() as u64 || location.y >= self.data.len() as u64 {
            return Err(anyhow!("Pixel out of bounds"));
        }
        self.data[location.y as usize][location.x as usize] = PixelWithLocation { pixel, location };
        Ok(())
    }
    pub fn get_pixel(&self, location: XY) -> Result<PixelWithLocation, Error> {
        if location.x >= self.data[0].len() as u64 || location.y >= self.data.len() as u64 {
            return Err(anyhow!("Pixel out of bounds"));
        }
        Ok(self.data[location.y as usize][location.x as usize].clone())
    }
    pub fn set_user(&mut self, user: User) -> Result<(), Error> {
        self.users.insert(user.id.clone(), user);
        Ok(())
    }
    pub fn get_user(&self, id: String) -> Option<User> {
        // println!("Users: {:?}", self.users);
        self.users.get(&id).cloned()
    }
}

pub fn hash(s: String) -> String {
    // let mut hasher = Sha256::new();
    // hasher.update(s);
    // let result = hasher.finalize();
    // format!("{:x}", result)
    s
}

#[derive(Deserialize, Default, Debug, Serialize, Clone, PartialEq, Eq)]
pub struct PixelWithLocation {
    pub pixel: MaybePixel,
    pub location: XY,
}

impl PixelWithLocation {
    pub fn new(pixel: MaybePixel, location: XY) -> PixelWithLocation {
        PixelWithLocation { pixel, location }
    }
}

#[derive(Deserialize, Default, Debug, Serialize, Clone, PartialEq, Eq)]
pub struct GenericPixelWithLocation {
    pub color: Color,
    pub location: XY,
}

impl GenericPixelWithLocation {
    pub fn into_full(self, artist_id: String) -> PixelWithLocation {
        PixelWithLocation {
            pixel: MaybePixel::Pixel(Pixel { color: self.color, artist_id }),
            location: self.location,
        }
    }
}

#[derive(Deserialize, Default, Debug, Serialize, Clone, PartialEq, Eq)]
pub enum MaybePixel {
    Pixel(Pixel),
    #[default]
    None,
}

#[derive(Deserialize, Default, Debug, Serialize, Clone, PartialEq, Eq)]
pub struct Pixel {
    pub color: Color,
    pub artist_id: String,
}
#[derive(Deserialize, Default, Debug, Serialize, Clone, PartialEq, Eq, Copy)]
pub struct XY {
    pub x: u64,
    pub y: u64,
}

impl XY {
    pub fn from_nested_vec(vec: &Vec<Vec<PixelWithLocation>>) -> Result<XY, Error> {
        if vec.is_empty() {
            return Err(anyhow!("Empty vec"));
        }
        let y = vec.len() as u64;
        let x = vec[0].len() as u64;
        for row in vec {
            if row.len() != x as usize {
                return Err(anyhow!("Uneven vec"));
            }
        }
        Ok(XY { x, y })
    }
}
#[derive(Deserialize, Default, Debug, Serialize, Clone, PartialEq, Eq, Copy)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}
#[derive(Deserialize, Default, Debug, Serialize, Clone, PartialEq, Eq)]
pub struct User {
    pub name: String,
    pub id: String,
}

#[async_trait::async_trait]
pub trait PlaceInterface: Send {
    async fn load(&self) -> Result<Place, Error>;
    async fn save_all(&self, place: &Place) -> Result<(), Error>;
    async fn save_pixel(&self, pixel: &PixelWithLocation) -> Result<(), Error>;
    async fn save_user(&self, user: &User) -> Result<(), Error>;
}

pub struct MetaPlace {
    pub place: Place,
    pub interface: Box<dyn PlaceInterface>,
    pub websockets: Vec<WebsocketHandler>,
}

impl std::fmt::Debug for MetaPlace {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MetaPlace").field("img", &self.place).field("interface", &"Box<dyn PlaceInterface>").finish()
    }
}

impl MetaPlace {
    pub fn add_websocket(&mut self, id: String, handle: UnboundedSender<ToClientMsg>) -> Option<User> {
        self.websockets.push(WebsocketHandler {
            id: id.clone(),
            handle,
            closed: false,
        });
        self.place.users.iter().find(|(uid, _)| uid == &&id).map(|(_, user)| user.clone())
    }
    pub fn remove_websocket(&mut self, id: &str) {
        self.websockets.retain(|ws| ws.id != id);
    }
    pub async fn new(interface: Box<dyn PlaceInterface>) -> Result<MetaPlace, Error> {
        let place = interface.load().await?;
        Ok(MetaPlace { place, interface, websockets: vec![] })
    }
    pub async fn save_pixel(&mut self, pixel: &PixelWithLocation) -> Result<(), Error> {
        self.interface.save_pixel(pixel).await?;
        self.place.data[pixel.location.y as usize][pixel.location.x as usize] = pixel.clone();
        Ok(())
    }
    pub async fn save(&mut self) -> Result<(), Error> {
        self.interface.save_all(&self.place).await
    }
    pub fn update_username(&mut self, id: &str, name: &str, nonce: Option<String>) -> Result<(), Error> {
        let user = self.place.users.get_mut(id).ok_or_else(|| anyhow!("No user with id {} found", id))?;
        user.name = name.to_string();
        // emit user update to all clients
        self.websockets.retain(|ws| !ws.closed);
        for ws in self.websockets.iter_mut() {
            let r = if ws.id == user.id {
                ws.handle.send(ToClientMsg::UserUpdate(nonce.clone(), user.clone()))
            } else {
                ws.handle.send(ToClientMsg::UserUpdate(None, user.clone()))
            };
            if r.is_err() {
                ws.closed = true;
            }
        }
        self.websockets.retain(|ws| !ws.closed);
        Ok(())
    }
    pub fn add_user(&mut self, user: User, nonce: Option<String>) -> Result<(), Error> {
        self.place.users.insert(user.id.clone(), user.clone());
        // emit user update to all clients
        self.websockets.retain(|ws| !ws.closed);
        for ws in self.websockets.iter_mut() {
            let r = if ws.id == user.id {
                ws.handle.send(ToClientMsg::UserUpdate(nonce.clone(), user.clone()))
            } else {
                ws.handle.send(ToClientMsg::UserUpdate(None, user.clone()))
            };
            if r.is_err() {
                ws.closed = true;
            }
        }
        self.websockets.retain(|ws| !ws.closed);
        Ok(())
    }
    pub fn update_pixel(&mut self, pixel: &PixelWithLocation, nonce: Option<String>) -> Result<(), Error> {
        if let MaybePixel::Pixel(pnes) = &pixel.pixel {
            let row = self
                .place
                .data
                .get_mut(pixel.location.y as usize)
                .ok_or_else(|| anyhow!("Index {} on y out of bounds", pixel.location.y))?;
            let p = row.get_mut(pixel.location.x as usize).ok_or_else(|| anyhow!("Index {} on x out of bounds", pixel.location.x))?;
            if let (MaybePixel::Pixel(j), MaybePixel::Pixel(k)) = (p.pixel.clone(), pixel.pixel.clone()) {
                if j.color == k.color {
                    return Err(anyhow!("Pixel already has color"));
                }
            }
            // match p.pixel.clone() {
            //     MaybePixel::Pixel(paxel) => {
            //         match paxel {
            //             MaybePixel::Pixel(puxel) => {
            //                 if paxel.color == puxel.color {
            //                     return Ok(());
            //                 }
            //             }
            //         }
            //     }
            //     MaybePixel::None => {}
            // }
            *p = pixel.clone();
            // emit pixel update to all clients
            self.websockets.retain(|ws| !ws.closed);
            for ws in self.websockets.iter_mut() {
                let r = if ws.id == pnes.artist_id {
                    ws.handle.send(ToClientMsg::PixelUpdate(nonce.clone(), pixel.clone()))
                } else {
                    ws.handle.send(ToClientMsg::PixelUpdate(None, pixel.clone()))
                };
                if r.is_err() {
                    ws.closed = true;
                }
            }
            self.websockets.retain(|ws| !ws.closed);
            Ok(())
        } else {
            Err(anyhow!("Pixel is not a pixel"))
        }
    }
    pub fn send_chat_msg(&mut self, msg: ChatMsg, nonce: Option<String>) -> Result<(), Error> {
        // emit chat msg to all clients
        self.websockets.retain(|ws| !ws.closed);
        for ws in self.websockets.iter_mut() {
            let r = if ws.id == msg.user_id {
                ws.handle.send(ToClientMsg::ChatMsg(nonce.clone(), msg.clone()))
            } else {
                ws.handle.send(ToClientMsg::ChatMsg(None, msg.clone()))
            };
            if r.is_err() {
                ws.closed = true;
            }
        }
        self.websockets.retain(|ws| !ws.closed);
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ChatMsg {
    pub user_id: String,
    pub msg: String,
}

pub fn program_path() -> Result<std::path::PathBuf, Error> {
    Ok(dirs::config_dir().ok_or_else(|| anyhow!("No config Dir found"))?.join("place_rs"))
}

pub struct WebsocketHandler {
    pub id: String,
    pub handle: UnboundedSender<ToClientMsg>,
    pub closed: bool,
}

pub fn safe_get_from_terminal<T: std::str::FromStr>(name: &str) -> T {
    loop {
        println!("{}:", name);
        let mut input = String::new();
        std::io::stdin().read_line(&mut input).unwrap();
        match input.trim().parse::<T>() {
            Ok(val) => return val,
            Err(_) => println!("Please enter a valid {}!", std::any::type_name::<T>()),
        }
    }
}
