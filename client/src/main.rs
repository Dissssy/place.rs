use anyhow::Error;
use async_std::io::prelude::*;
use async_std::io::stdin;
use async_std::io::BufReader;
use futures_util::TryStreamExt;
// use async_std::stream::StreamExt;
use futures_util::stream::FuturesUnordered;
// use futures_util::StreamExt;
// mod place;
use lazy_static::lazy_static;
mod config;
use config::Config;
use place_rs_shared::messages::ToClientMsg;
use place_rs_shared::messages::ToServerMsg;
use tokio::sync::oneshot::Receiver;
use websocket::Callback;

use crate::websocket::RX;
mod websocket;

lazy_static! {
    pub static ref CONFIG: Config = Config::new().unwrap();
}

#[tokio::main]
async fn main() {
    let mut ws = websocket::Websocket::new(CONFIG.get_ws_url());
    let stdin = BufReader::new(stdin());
    let mut lines = stdin.lines();
    let mut generic = match ws.connect().await {
        Ok(x) => x,
        Err(e) => {
            panic!("Error connecting to server: {}", e)
        }
    };
    println!("Connected to server");
    let mut nonce = 0;
    let mut futures: FuturesUnordered<Receiver<Result<Callback, Error>>> = FuturesUnordered::new();
    loop {
        tokio::select! {
            r = futures.try_next() => {
                match r {
                    Err(e) => {
                        println!("Error: {}", e);
                    }
                    Ok(None) => {}
                    Ok(Some(Ok(x))) => {
                        println!("Promise resolved");
                        match x {
                            Callback::Message(_, RX::Message(ToClientMsg::ChatMsg(_, msg))) => {
                                println!("{}\t: {}", msg.user_id, msg.msg);
                            }
                            _ => {
                                println!("Received message: {:?}", x);
                            }
                        }
                    }
                    Ok(Some(Err(e))) => {
                        println!("Websocket error: {}", e);
                    }
                }
            }
            r = lines.try_next() => {
                match r {
                    Err(e) => {
                        println!("Error: {}", e);
                    }
                    Ok(None) => {}
                    Ok(Some(x)) => {
                        if x == "help" {
                            println!("Available commands:");
                            println!("help - show this message");
                            println!("msg <message> - send a message to the other clients");
                            println!("setpixel <x> <y> <r> <g> <b> - set pixel at <x>, <y> to color <r>, <g>, <b>");
                            println!("setname <name> - set your name to <name>");
                            println!("heartbeat - send a heartbeat to the server");
                            println!("requestplace - crash the program after like 3 seconds :)");
                        } else {
                            let args = x.split_whitespace().map(|x| x.to_string()).collect::<Vec<String>>();
                            let thisnonce = format!("{}|{}", std::process::id(), nonce);
                            let wscommand = ToServerMsg::parse(args, thisnonce.clone());
                            match wscommand {
                                Ok(command) => {
                                    futures.push(ws.send(command, thisnonce).await.unwrap());
                                    nonce += 1;
                                }
                                Err(e) => {
                                    println!("Error: {}", e);
                                }
                            }
                        }
                    }
                }
            }
            Some(r) = generic.recv() => {
                match r {
                    RX::Message(ToClientMsg::ChatMsg(_, msg)) => {
                        println!("{}\t: {}", msg.user_id, msg.msg);
                    }
                    _ => {
                        println!("Received message: {:?}", r);
                    }
                }
            }
        }
    }
}
