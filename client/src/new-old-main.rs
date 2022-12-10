#![feature(map_try_insert)]
use anyhow::Error;
use async_std::io::prelude::*;
use async_std::io::stdin;
use async_std::io::BufReader;
use futures_util::StreamExt;
use futures_util::TryStreamExt;
// use async_std::stream::StreamExt;
use futures_util::stream::FuturesUnordered;
// use futures_util::StreamExt;
mod place;
use lazy_static::lazy_static;
mod config;
use config::Config;
use place::Place;
use place_rs_shared::messages::ToClientMsg;
use place_rs_shared::messages::ToServerMsg;
use place_rs_shared::Color;
use place_rs_shared::XY;
use tokio::sync::oneshot::Receiver;
use websocket::Callback;

use crate::websocket::RX;
mod websocket;

lazy_static! {
    pub static ref CONFIG: Config = Config::new().unwrap();
}

#[tokio::main]
async fn main() {
    let mut place = Place::new(CONFIG.get_ws_url(), CONFIG.get_http_url()).await.unwrap();
    println!("Connected to server");
    //     let mut ws = websocket::Websocket::new(CONFIG.get_ws_url());
    let stdin = BufReader::new(stdin());
    let mut lines = stdin.lines();
    //     let mut generic = match ws.connect().await {
    //         Ok(x) => x,
    //         Err(e) => {
    //             panic!("Error connecting to server: {}", e)
    //         }
    //     };
    //     println!("Connected to server");
    //     let mut nonce = 0;
    //     let mut futures: FuturesUnordered<Receiver<Result<Callback, Error>>> = FuturesUnordered::new();
    loop {
        //         tokio::select! {
        //             r = futures.try_next() => {
        //                 match r {
        //                     Err(e) => {
        //                         println!("Error: {}", e);
        //                     }
        //                     Ok(None) => {}
        //                     Ok(Some(Ok(x))) => {
        //                         println!("Promise resolved");
        //                         match x {
        //                             Callback::Message(_, RX::Message(ToClientMsg::ChatMsg(_, msg))) => {
        //                                 println!("{}\t: {}", msg.user_id, msg.msg);
        //                             }
        //                             _ => {
        //                                 println!("Received message: {:?}", x);
        //                             }
        //                         }
        //                     }
        //                     Ok(Some(Err(e))) => {
        //                         println!("Websocket error: {}", e);
        //                     }
        //                 }
        //             }
        //             r = lines.try_next() => {
        match lines.next().await {
            Some(Ok(x)) => {
                if x == "help" {
                    println!("Available commands:");
                    println!("help - show this message");
                    println!("msg <message> - send a message to the other clients");
                    println!("setpixel <x> <y> <r> <g> <b> - set pixel at <x>, <y> to color <r>, <g>, <b>");
                    println!("setname <name> - set your name to <name>");
                    println!("heartbeat - send a heartbeat to the server");
                    println!("requestplace - crash the program after like 3 seconds :)");
                } else {
                    let mut args = x.split_whitespace().map(|x| x.to_string()).collect::<Vec<String>>().into_iter();
                    let command = match args.next() {
                        Some(x) => x,
                        None => continue,
                    };
                    match command.as_str() {
                        "paint" => {
                            // paint requires an x, y, r, g, b. attempt to get all, print error and continue if not
                            let x = match args.next() {
                                Some(x) => match x.parse::<u64>() {
                                    Ok(x) => x,
                                    Err(e) => {
                                        println!("Error: {}", e);
                                        continue;
                                    }
                                },
                                None => {
                                    println!("Error: x not specified");
                                    continue;
                                }
                            };
                            let y = match args.next() {
                                Some(x) => match x.parse::<u64>() {
                                    Ok(x) => x,
                                    Err(e) => {
                                        println!("Error: {}", e);
                                        continue;
                                    }
                                },
                                None => {
                                    println!("Error: y not specified");
                                    continue;
                                }
                            };
                            let r = match args.next() {
                                Some(x) => match x.parse::<u8>() {
                                    Ok(x) => x,
                                    Err(e) => {
                                        println!("Error: {}", e);
                                        continue;
                                    }
                                },
                                None => {
                                    println!("Error: r not specified");
                                    continue;
                                }
                            };
                            let g = match args.next() {
                                Some(x) => match x.parse::<u8>() {
                                    Ok(x) => x,
                                    Err(e) => {
                                        println!("Error: {}", e);
                                        continue;
                                    }
                                },
                                None => {
                                    println!("Error: g not specified");
                                    continue;
                                }
                            };
                            let b = match args.next() {
                                Some(x) => match x.parse::<u8>() {
                                    Ok(x) => x,
                                    Err(e) => {
                                        println!("Error: {}", e);
                                        continue;
                                    }
                                },
                                None => {
                                    println!("Error: b not specified");
                                    continue;
                                }
                            };
                            let r = place.paint(XY { x, y }, Color { r, g, b }).await;
                            if let Err(e) = r {
                                println!("Error: {}", e);
                            }
                        }
                        "msg" => {
                            // the rest of the args is the message
                            let msg = args.collect::<Vec<String>>().join(" ").trim().to_string();
                            let r = place.chat(msg).await;
                            if let Err(e) = r {
                                println!("Error: {}", e);
                            }
                        }
                        "nick" => {
                            // the rest of the args is the name
                            let name = args.collect::<Vec<String>>().join(" ").trim().to_string();
                            let r = place.set_username(name).await;
                            if let Err(e) = r {
                                println!("Error: {}", e);
                            }
                        }
                        "export" => {
                            let r = place.export_img().await;
                            if let Err(e) = r {
                                println!("Error: {}", e);
                            }
                        }
                        _ => {
                            println!("Unknown command");
                        }
                    }
                    // let args = x.split_whitespace().map(|x| x.to_string()).collect::<Vec<String>>();
                    // let thisnonce = format!("{}|{}", std::process::id(), nonce);
                    // let wscommand = ToServerMsg::parse(args, thisnonce.clone());
                    // match wscommand {
                    //     Ok(command) => {
                    //         // if command is requestplace instance
                    //         let thisnonce = match command {
                    //             ToServerMsg::RequestPlace(_) => {
                    //                 "binary".to_string()
                    //             }
                    //             _ => thisnonce
                    //         };
                    //         futures.push(ws.send(command, thisnonce).await.unwrap());
                    //         nonce += 1;
                    //     }
                    //     Err(e) => {
                    //         println!("Error: {}", e);
                    //         ws = websocket::Websocket::new(CONFIG.get_ws_url());
                    //         generic = match ws.connect().await {
                    //             Ok(x) => x,
                    //             Err(e) => {
                    //                 panic!("Error connecting to server: {}", e)
                    //             }
                    //         };
                    //     }
                    // }
                }
            }
            None => {
                println!("Error: stdin closed");
                break;
            }
            Some(Err(e)) => {
                println!("Error: {}", e);
                break;
            }
        }
        if place.changed().await {
            let r = place.export_img().await;
            if let Err(e) = r {
                println!("Error: {}", e);
            }
        }
        //             }
        //             Some(r) = generic.recv() => {
        //                 match r {
        //                     RX::Message(ToClientMsg::ChatMsg(_, msg)) => {
        //                         println!("{}\t: {}", msg.user_id, msg.msg);
        //                     }
        //                     _ => {
        //                         println!("Received message: {:?}", r);
        //                     }
        //                 }
        //             }
        //         }
    }
}
