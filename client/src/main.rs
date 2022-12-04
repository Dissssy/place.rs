use std::io;

use std::env;

use futures_util::SinkExt;
use futures_util::{future, pin_mut, StreamExt};
use place_rs_shared::RawWebsocketMessage;
use place_rs_shared::WebsocketMessage;
use place_rs_shared::{Color, Pixel, User, XY};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::{net::TcpStream, sync::mpsc::unbounded_channel};
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};
use tokio_tungstenite::{tungstenite::protocol::WebSocketConfig, MaybeTlsStream, WebSocketStream};

#[tokio::main]
async fn main() {
    let mut client = WebSocketHandler::new("wss://place.planetfifty.one/ws/").await.unwrap();
    loop {
        // read stdin for Command: <command> <args> <args> ...
        let mut input = String::new();
        io::stdin().read_line(&mut input).unwrap();
        let mut args = input.split_whitespace();
        let command = args.next().unwrap();
        let wscommand = match command {
            "paint" => {
                // paint <x> <y> <r> <g> <b>
                let x = args.next().unwrap().parse::<usize>().unwrap();
                let y = args.next().unwrap().parse::<usize>().unwrap();
                let r = args.next().unwrap().parse::<u8>().unwrap();
                let g = args.next().unwrap().parse::<u8>().unwrap();
                let b = args.next().unwrap().parse::<u8>().unwrap();
                TX::Paint(XY { x, y }, Color { r, g, b })
            }
            "nick" => {
                // nick <name>
                let name = args.next().unwrap();
                TX::Nick(name.to_string())
            }
            "help" => {
                println!("Commands: \n\tpaint <x> <y> <r> <g> <b> \n\tnick <name>");
                continue;
            }
            _ => {
                // unknown command
                println!("unknown command");
                continue;
            }
        };
        client.send(wscommand).await.unwrap();
    }
}

#[derive(Debug)]
enum TX {
    Paint(XY, Color),
    Nick(String),
}

#[derive(Debug)]
enum RX {
    Pixel(Pixel),
    User(User),
}

struct WebSocketHandler {
    handle: tokio::task::JoinHandle<()>,
    sender: tokio::sync::mpsc::UnboundedSender<TX>,
    receiver: tokio::sync::mpsc::UnboundedReceiver<RX>,
}

impl WebSocketHandler {
    async fn new(url: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let (mut ws_stream, _) = tokio_tungstenite::connect_async_with_config(url, None).await?;

        let listencommand = RawWebsocketMessage::Listen;
        ws_stream.send(Message::Text(serde_json::to_string(&listencommand).unwrap())).await?;

        let (ctx, mut crx) = unbounded_channel::<TX>();
        let (rtx, rrx) = unbounded_channel::<RX>();
        let handle = tokio::spawn(async move {
            loop {
                // here we will be checking for a message from the server, and sending a message to the server with a configurable interval
                tokio::select! {
                    Some(msg) = crx.recv() => {
                        // send message to server
                        match msg {
                            TX::Paint(xy, color) => {
                                let pixel = RawWebsocketMessage::PixelUpdate { location: xy, color };
                                let msg = serde_json::to_string(&pixel).unwrap();
                                ws_stream.send(tokio_tungstenite::tungstenite::Message::Text(msg)).await.unwrap();
                            }
                            TX::Nick(name) => {
                                let user = RawWebsocketMessage::SetUsername(name);
                                let msg = serde_json::to_string(&user).unwrap();
                                ws_stream.send(tokio_tungstenite::tungstenite::Message::Text(msg)).await.unwrap();
                            }
                        }
                    }
                    Some(msg) = ws_stream.next() => {
                        if let Ok(msg) = msg {
                            // println!("got message from server: {:?}", msg);
                            let msg = serde_json::from_str::<WebsocketMessage>(&msg.to_string()).unwrap();
                            match msg {
                                WebsocketMessage::Pixel(pixel) => {
                                    println!("Pixel: {:?}", pixel);
                                    rtx.send(RX::Pixel(pixel)).unwrap();
                                }
                                WebsocketMessage::User(user) => {
                                    println!("User: {:?}", user);
                                    rtx.send(RX::User(user)).unwrap();
                                }
                                WebsocketMessage::Heartbeat => {
                                    // send heartbeat back
                                    // println!("Heartbeat");
                                    let heartbeat = RawWebsocketMessage::Heartbeat;
                                    let msg = serde_json::to_string(&heartbeat).unwrap();
                                    ws_stream.send(tokio_tungstenite::tungstenite::Message::Text(msg)).await.unwrap();
                                }
                                WebsocketMessage::Listening => {
                                    // println!("Listening");
                                }
                                WebsocketMessage::Error(err) => {
                                    println!("Error: {:?}", err);
                                }
                            }
                        }
                    }
                }
            }
        });
        Ok(Self { handle, sender: ctx, receiver: rrx })
    }
    async fn send(&mut self, msg: TX) -> Result<(), Box<dyn std::error::Error>> {
        self.sender.send(msg).unwrap();
        Ok(())
    }
}
