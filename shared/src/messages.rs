use anyhow::{anyhow, Error};
use serde::{Deserialize, Serialize};

use crate::{Color, GenericPixelWithLocation, PixelWithLocation, User, XY};

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default)]
pub enum ToClientMsg {
    PixelUpdate(PixelWithLocation),
    UserUpdate(User),
    #[default]
    Heartbeat,
    GenericError(String),
    TimeoutError(TimeoutType),
}
// NOTE: Client may also receive BINARY data, which is a compressed Place struct

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum TimeoutType {
    Username(u64),
    Pixel(u64),
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default)]
pub enum ToServerMsg {
    SetPixel(GenericPixelWithLocation),
    SetName(String),
    #[default]
    Heartbeat,
    RequestPlace,
}

impl ToServerMsg {
    pub fn parse(args: Vec<String>) -> Result<Self, Error> {
        let mut args = args.into_iter();
        let cmd = args.next().ok_or_else(|| anyhow!("No command"))?;
        match cmd.as_str() {
            "setpixel" => Ok(ToServerMsg::SetPixel(GenericPixelWithLocation {
                location: XY {
                    x: args.next().ok_or_else(|| anyhow!("No x"))?.parse()?,
                    y: args.next().ok_or_else(|| anyhow!("No y"))?.parse()?,
                },
                color: Color {
                    r: args.next().ok_or_else(|| anyhow!("No r"))?.parse()?,
                    g: args.next().ok_or_else(|| anyhow!("No g"))?.parse()?,
                    b: args.next().ok_or_else(|| anyhow!("No b"))?.parse()?,
                },
            })),
            "setname" => Ok(ToServerMsg::SetName(args.next().ok_or_else(|| anyhow!("No name"))?)),
            "heartbeat" => Ok(ToServerMsg::Heartbeat),
            "requestplace" => Ok(ToServerMsg::RequestPlace),
            _ => Err(anyhow!("Unknown command")),
        }
    }
}
