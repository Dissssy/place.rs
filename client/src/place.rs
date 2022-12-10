use std::sync::Arc;

use anyhow::Error;
use place_rs_shared::Place as RawPlace;
use tokio::sync::Mutex;

use crate::websocket::Websocket;

pub struct Place {
    place: Arc<Mutex<RawPlace>>,
    websocket: Websocket,
    rest_url: String,
    ws_url: String,
}

impl Place {
    pub async fn new(ws_url: String, rest_url: String) -> Result<Self, Error> {
        let websocket = Websocket::new(ws_url.clone());
        websocket.connect().await?;
        let place = websocket.get_place().await?;
        Ok(Self {
            place: Arc::new(Mutex::new(place)),
            websocket,
            rest_url,
            ws_url,
        })
    }
}
