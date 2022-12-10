use anyhow::{anyhow, Error};
use serde::{Deserialize, Serialize};

use crate::{ChatMsg, Color, GenericPixelWithLocation, PixelWithLocation, User, XY};

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum ToClientMsg {
    PixelUpdate(Option<String>, PixelWithLocation),
    UserUpdate(Option<String>, User),
    Heartbeat(Option<String>),
    GenericError(Option<String>, String),
    TimeoutError(Option<String>, TimeoutType),
    ChatMsg(Option<String>, ChatMsg),
}
// NOTE: Client may also receive BINARY data, which is a compressed Place struct

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum TimeoutType {
    Username(u64),
    Pixel(u64),
    Chat(u64),
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum ToServerMsg {
    SetPixel(String, GenericPixelWithLocation),
    SetName(String, String),
    Heartbeat(String),
    RequestPlace(String),
    ChatMsg(String, String),
}

impl ToServerMsg {
    pub fn parse(args: Vec<String>, nonce: String) -> Result<Self, Error> {
        let mut args = args.into_iter();
        let cmd = args.next().ok_or_else(|| anyhow!("No command"))?;
        match cmd.as_str() {
            "setpixel" => Ok(ToServerMsg::SetPixel(
                nonce,
                GenericPixelWithLocation {
                    location: XY {
                        x: args.next().ok_or_else(|| anyhow!("No x"))?.parse()?,
                        y: args.next().ok_or_else(|| anyhow!("No y"))?.parse()?,
                    },
                    color: Color {
                        r: args.next().ok_or_else(|| anyhow!("No r"))?.parse()?,
                        g: args.next().ok_or_else(|| anyhow!("No g"))?.parse()?,
                        b: args.next().ok_or_else(|| anyhow!("No b"))?.parse()?,
                    },
                },
            )),
            "setname" => Ok(ToServerMsg::SetName(nonce, args.next().ok_or_else(|| anyhow!("No name"))?)),
            "heartbeat" => Ok(ToServerMsg::Heartbeat(nonce)),
            "requestplace" => Ok(ToServerMsg::RequestPlace("binary".to_string())),
            "msg" => Ok(ToServerMsg::ChatMsg(nonce, args.collect::<Vec<String>>().join(" ").trim().to_string())),
            _ => Err(anyhow!("Unknown command")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]

pub struct SafeInfo {
    pub size: XY,
}

impl SafeInfo {
    pub fn new(size: XY) -> Self {
        Self { size }
    }
}
