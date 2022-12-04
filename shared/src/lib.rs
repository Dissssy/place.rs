use anyhow::{anyhow, Error};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sqlx::{postgres::PgPoolOptions, Pool, Postgres, Row};
use std::{
    fmt::{Display, Formatter},
    io::Write,
    path::PathBuf,
    str::FromStr,
};
use tokio::sync::mpsc::UnboundedSender;

pub struct Place {
    pub data: Vec<Vec<Pixel>>,

    pub users: Vec<User>,
    pub handler: Box<dyn Handler>,
    pub websockets: Vec<WebsocketHandler>,
    pub config: ServerConfig,
}

impl Clone for Place {
    fn clone(&self) -> Self {
        Self {
            data: self.data.clone(),
            users: self.users.clone(),
            handler: self.handler.clone(),
            websockets: vec![],
            config: self.config.clone(),
        }
    }
}

pub struct WebsocketHandler {
    pub id: String,
    pub handle: UnboundedSender<WebsocketMessage>,
    pub closed: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum WebsocketMessage {
    Pixel(Pixel),
    User(User),
    Heartbeat,
    Listening,
    Error(String),
}

impl Place {
    pub async fn load() -> Result<Self, Error> {
        let config = ServerConfig::load()?;
        let p: Box<dyn Handler> = match config.handler {
            HandlerType::Json => Box::new(JsonHandler::new()),
            HandlerType::Postgres(data) => Box::new(PostgresHandler {
                pool: PgPoolOptions::new().max_connections(5).connect(&get_pg_uri(data.clone())).await?,
                data,
            }),
            // _ => unreachable!("Handler not implemented"),
        };
        let p = p.load().await;
        if let Ok(p) = p {
            Ok(p)
        } else {
            Err(p.err().unwrap())
        }
    }
    pub fn get_save_data(&self) -> SaveData {
        SaveData {
            data: self.data.clone(),
            users: self.users.clone(),
        }
    }
    pub fn get_chunk(&self, x: usize, y: usize) -> Vec<Pixel> {
        let mut chunk: Vec<Pixel> = Vec::new();
        for i in 0..self.config.chunk_size {
            for j in 0..self.config.chunk_size {
                let pixel = self.data.get(x * self.config.chunk_size + i).and_then(|row| row.get(y * self.config.chunk_size + j));
                if let Some(pixel) = pixel {
                    chunk.push(pixel.clone());
                }
            }
        }
        chunk
    }
    pub async fn set_pixel(&mut self, newpixel: Pixel) -> Result<(), Error> {
        if newpixel.user == LazyUser::None {
            return Err(anyhow!("User not found"));
        }
        let now = chrono::Utc::now();
        let thisnow = now.timestamp();
        let user = self.users.iter_mut().find(|user| match newpixel.user.clone() {
            LazyUser::User(theuser) => theuser.id == user.id,
            LazyUser::Id(id) => id == user.id,
            LazyUser::None => unreachable!("User not found"),
        });
        if let Some(user) = user.as_ref() {
            if user.timeout > thisnow {
                return Err(anyhow::anyhow!("User is within timeout. Please wait {} seconds", user.timeout - thisnow));
            }
        }

        let row = self.data.get_mut(newpixel.location.x).ok_or_else(|| anyhow::anyhow!("Pixel x out of bounds"))?;
        let pixel = row.get_mut(newpixel.location.y).ok_or_else(|| anyhow::anyhow!("Pixel y out of bounds"))?;

        if pixel.color == newpixel.color {
            return Err(anyhow::anyhow!("Pixel color is the same, paint not wasted"));
        }

        *pixel = newpixel.clone();

        for websocket in self.websockets.iter_mut() {
            let r = websocket.handle.send(WebsocketMessage::Pixel(newpixel.clone()));
            if r.is_err() {
                websocket.closed = true;
            }
        }
        self.websockets.retain(|websocket| !websocket.closed);

        if let Some(user) = user {
            user.timeout = thisnow + self.config.timeout;
        } else {
            let mut thisuser = match newpixel.user.clone() {
                LazyUser::User(theuser) => theuser,
                LazyUser::Id(id) => self.users.iter().find(|user| user.id == id).ok_or_else(|| anyhow::anyhow!("User not found"))?.clone(),
                LazyUser::None => unreachable!("User not found"),
            };
            thisuser.timeout = thisnow + self.config.timeout;
            self.users.push(thisuser);
        }
        self.save_pixel(newpixel).await?;
        Ok(())
    }
    pub fn add_websocket(&mut self, id: String, handle: UnboundedSender<WebsocketMessage>) -> Option<User> {
        self.websockets.push(WebsocketHandler {
            id: id.clone(),
            handle,
            closed: false,
        });
        self.users.iter().find(|user| user.id == id).cloned()
    }
    pub fn remove_websocket(&mut self, id: String) {
        self.websockets.retain(|websocket| websocket.id != id);
    }
    pub async fn save_pixel(&self, pixel: Pixel) -> Result<(), Error> {
        self.handler.save_pixel(pixel).await
    }
    pub async fn set_username(&mut self, id: String, username: String) -> Result<(), Error> {
        let user = self.users.iter_mut().find(|user| user.id == id);
        if let Some(user) = user {
            user.name = username;
            // iter over all pixels and change the username if it matches the id
            for row in self.data.iter_mut() {
                for pixel in row.iter_mut() {
                    if let LazyUser::Id(id) = pixel.user.clone() {
                        if id == user.id {
                            pixel.user = LazyUser::User(user.clone());
                        }
                    }
                }
            }
            for websocket in self.websockets.iter_mut() {
                let r = websocket.handle.send(WebsocketMessage::User(user.clone()));
                if r.is_err() {
                    websocket.closed = true;
                }
            }
            self.websockets.retain(|websocket| !websocket.closed);
            Ok(())
        } else {
            Err(anyhow!("User not found"))
        }
    }
}

fn get_blank_data(size: XY) -> Vec<Vec<Pixel>> {
    let mut data = Vec::new();
    for y in 0..size.y {
        let mut row = Vec::new();
        for x in 0..size.x {
            row.push(Pixel::new(x, y));
        }
        data.push(row);
    }
    data
}

pub struct JsonHandler;

#[async_trait]
impl Handler for JsonHandler {
    async fn load(&self) -> Result<Place, Error> {
        let mut path = get_data_path();
        path.push("data.json");
        let config = ServerConfig::load()?;

        if !path.exists() {
            let data = get_blank_data(get_size());
            let place = Place {
                data,
                users: vec![],
                handler: Box::new(Self::new()),
                websockets: Vec::new(),
                config,
            };

            let p = place.handler.save(place.get_save_data());
            if p.await.is_ok() {
                Ok(place)
            } else {
                Err(anyhow!("Error saving"))
            }
        } else {
            let file = std::fs::File::open(path)?;
            let data: SaveData = serde_json::from_reader(file)?;
            Ok(Place {
                data: data.data,
                users: data.users,
                handler: Box::new(Self::new()),
                websockets: Vec::new(),
                config,
            })
        }
    }
    async fn save(&self, data: SaveData) -> Result<(), Error> {
        let mut path = get_data_path();
        path.push("data.json");
        let file = std::fs::File::create(path)?;
        serde_json::to_writer_pretty(file, &data)?;
        Ok(())
    }
    async fn save_pixel(&self, _pixel: Pixel) -> Result<(), Error> {
        Ok(())
    }
    fn clone(&self) -> Box<dyn Handler> {
        Box::new(Self::new())
    }
}

pub struct PostgresHandler {
    data: PostgresConfig,
    pool: Pool<Postgres>,
}

fn get_pg_uri(data: PostgresConfig) -> String {
    format!("postgres://{}:{}@{}:{}/{}", data.username, data.password, data.host, data.port, data.database)
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub enum LazyUser {
    Id(String),
    User(User),
    None,
}

#[async_trait]
impl Handler for PostgresHandler {
    async fn save(&self, _data: SaveData) -> Result<(), Error> {
        Ok(())
    }

    async fn load(&self) -> Result<Place, Error> {
        println!("Loading from postgres");
        let config = ServerConfig::load()?;

        let mut data = get_blank_data(config.size);

        let mut users = Vec::new();
        println!("Loading data");
        let mut count = 0;
        let rows = sqlx::query("SELECT * FROM data").fetch_all(&self.pool).await?;
        for row in rows {
            let x: i32 = row.get("x");
            let x = x as usize;
            let y: i32 = row.get("y");
            let y = y as usize;
            // let timeout: i32 = row.get("user_timeout");
            // let timeout = timeout as i64;
            let rtimestamp: i32 = row.get("timestamp");
            let timestamp = Some(rtimestamp as i64);

            let trow = data.get_mut(x);
            if let Some(trow) = trow {
                let tpixel = trow.get_mut(y);
                if let Some(tpixel) = tpixel {
                    let id: String = row.get("user_id");
                    let pixel = Pixel {
                        location: XY { x, y },
                        color: Color::from_str(row.get("color"))?,
                        user: LazyUser::Id(id.clone()),
                        timestamp,
                    };
                    let user = User {
                        id,
                        timeout: rtimestamp as i64,
                        name: row.get("user_name"),
                        username_timeout: rtimestamp as i64,
                    };
                    users.push(user);
                    *tpixel = pixel.clone();
                    count += 1;
                }
            }
        }
        println!("Loaded {} pixels", count);

        users.sort_by(|a, b| b.timeout.cmp(&a.timeout));
        users.dedup_by(|a, b| a.id == b.id);
        println!("Loaded {} users", users.len());
        // unlazify users
        for row in data.iter_mut() {
            for pixel in row.iter_mut() {
                if let LazyUser::Id(id) = &pixel.user {
                    if let Some(user) = users.iter().find(|user| user.id == *id) {
                        pixel.user = LazyUser::User(user.clone());
                    }
                }
            }
        }
        Ok(Place {
            data,
            users,
            handler: self.clone(),
            websockets: Vec::new(),
            config,
        })
    }

    async fn save_pixel(&self, pixel: Pixel) -> Result<(), Error> {
        let user = match &pixel.user {
            LazyUser::User(user) => user,
            _ => return Err(anyhow!("Invalid user")),
        };
        sqlx::query("INSERT INTO data (x, y, color, user_id, user_name, user_timeout, timestamp, uuid) VALUES ($1, $2, $3, $4, $5, $6, $7, $8) ON CONFLICT (uuid) DO UPDATE SET color = $3, user_id = $4, user_name = $5, user_timeout = $6, timestamp = $7").bind(pixel.location.x as i64).bind(pixel.location.y as i64).bind(&(pixel.color.to_string())).bind(&user.id).bind(&user.name).bind(user.timeout).bind(pixel.timestamp).bind(format!("{}:{}", pixel.location.x, pixel.location.y)).execute(&self.pool).await?;
        Ok(())
    }

    fn clone(&self) -> Box<dyn Handler> {
        Box::new(Self {
            data: self.data.clone(),
            pool: self.pool.clone(),
        })
    }
}

pub fn get_size() -> XY {
    let config = ServerConfig::load().unwrap();
    config.size
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaveData {
    pub data: Vec<Vec<Pixel>>,
    pub users: Vec<User>,
}

impl JsonHandler {
    pub fn new() -> Self {
        Self
    }
}

impl Default for JsonHandler {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pixel {
    pub color: Color,

    pub user: LazyUser,
    pub location: XY,
    pub timestamp: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Copy)]
pub struct XY {
    pub x: usize,
    pub y: usize,
}

impl Pixel {
    fn new(x: usize, y: usize) -> Self {
        Self {
            color: Color::default(),
            user: LazyUser::None,
            location: XY { x, y },
            timestamp: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct User {
    pub name: String,
    pub id: String,
    pub timeout: i64,
    pub username_timeout: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl FromStr for Color {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut chars = s.chars();
        let r = u8::from_str_radix(&chars.by_ref().take(2).collect::<String>(), 16)?;
        let g = u8::from_str_radix(&chars.by_ref().take(2).collect::<String>(), 16)?;
        let b = u8::from_str_radix(&chars.by_ref().take(2).collect::<String>(), 16)?;
        Ok(Self { r, g, b })
    }
}

impl Display for Color {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub size: XY,
    pub handler: HandlerType,
    pub timeout: i64,
    pub chunk_size: usize,
    pub username_timeout: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ServerConfigGetting {
    host: Option<String>,
    port: Option<u16>,
    size: Option<XY>,
    handler: Option<HandlerType>,
    timeout: Option<i64>,
    chunk_size: Option<usize>,
    username_timeout: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]

pub enum HandlerType {
    Json,
    Postgres(PostgresConfig),
}

#[async_trait]
pub trait Handler: Send {
    async fn save(&self, data: SaveData) -> Result<(), Error>;
    async fn load(&self) -> Result<Place, Error>;
    async fn save_pixel(&self, pixel: Pixel) -> Result<(), Error>;
    fn clone(&self) -> Box<dyn Handler>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostgresConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub database: String,
}

impl FromStr for HandlerType {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "json" => Ok(HandlerType::Json),
            "postgres" => Ok(HandlerType::Postgres(PostgresConfig {
                host: safe_getter(None, "Postgres host: "),
                port: safe_getter(None, "Postgres port: "),
                username: safe_getter(None, "Postgres username: "),
                password: safe_getter(None, "Postgres password: "),
                database: safe_getter(None, "Postgres database: "),
            })),
            _ => Err(anyhow::anyhow!("Invalid handler")),
        }
    }
}

impl ServerConfig {
    pub fn load() -> Result<Self, Error> {
        let mut config_path = get_data_path();

        std::fs::create_dir_all(&config_path)?;
        config_path.push("server_config.json");

        let config_getting: ServerConfigGetting = match std::fs::read_to_string(&config_path) {
            Ok(config) => match serde_json::from_str::<ServerConfigGetting>(&config) {
                Ok(config) => config,
                Err(e) => {
                    eprintln!("Error deserializing server config: {}", e);
                    ServerConfigGetting {
                        host: None,
                        port: None,
                        handler: None,
                        timeout: None,
                        chunk_size: None,
                        size: None,
                        username_timeout: None,
                    }
                }
            },
            Err(e) => {
                eprintln!("Error reading server config: {}", e);
                ServerConfigGetting {
                    host: None,
                    port: None,
                    handler: None,
                    timeout: None,
                    chunk_size: None,
                    size: None,
                    username_timeout: None,
                }
            }
        };
        let item = Self {
            host: safe_getter(config_getting.host, "Input host: "),
            port: safe_getter(config_getting.port, "Input port: "),
            handler: safe_getter(config_getting.handler, "Input handler: "),
            timeout: safe_getter(config_getting.timeout, "Input timeout: "),
            chunk_size: safe_getter(config_getting.chunk_size, "Input chunk size: "),
            size: XY {
                x: safe_getter(config_getting.size.map(|x| x.x), "Input width: "),
                y: safe_getter(config_getting.size.map(|x| x.y), "Input height: "),
            },
            username_timeout: safe_getter(config_getting.username_timeout, "Input username timeout: "),
        };

        let mut file = std::fs::File::create(&config_path)?;
        file.write_all(serde_json::to_string_pretty(&item)?.as_bytes())?;
        Ok(item)
    }
}

fn safe_getter<T: std::str::FromStr>(value: Option<T>, prompt: &str) -> T {
    match value {
        Some(value) => value,
        None => {
            let mut input = String::new();
            loop {
                print!("{}", prompt);
                std::io::stdout().flush().unwrap();
                std::io::stdin().read_line(&mut input).unwrap();
                match input.trim().parse::<T>() {
                    Ok(value) => break value,
                    Err(_) => {
                        println!("Invalid input");
                        input.clear();
                    }
                }
            }
        }
    }
}

fn get_data_path() -> PathBuf {
    dirs::config_dir().unwrap_or_else(|| PathBuf::from_str("./").unwrap()).join("place_rs")
}

#[derive(Deserialize, Debug, Serialize)]
pub enum RawWebsocketMessage {
    PixelUpdate { location: XY, color: Color },
    Heartbeat,
    Error { message: String },
    Listen,
    SetUsername(String),
}
