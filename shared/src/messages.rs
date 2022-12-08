use serde::{Deserialize, Serialize};

use crate::{GenericPixelWithLocation, PixelWithLocation, User};

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
