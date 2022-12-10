use async_std::io::prelude::*;
use async_std::io::stdin;
use async_std::io::BufReader;
use async_std::stream::StreamExt;
// mod place;
use lazy_static::lazy_static;
mod config;
use config::Config;
use place_rs_shared::messages::ToServerMsg;
mod websocket;

lazy_static! {
    pub static ref CONFIG: Config = Config::new().unwrap();
}

#[tokio::main]
async fn main() {
    let mut ws = websocket::Websocket::new(CONFIG.get_ws_url());
    let stdin = BufReader::new(stdin());
    let mut lines = stdin.lines();
    let _ = ws.connect().await;
    println!("Connected to server");
    let mut nonce = 0;
    loop {
        tokio::select! {
            // m = ws.recv() => {
            //     match m {
            //         Some(msg) => {
            //             println!("Received: {:?}", msg);
            //         }
            //         None => {
            //             println!("Disconnected");
            //             tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
            //             ws = websocket::Websocket::new(CONFIG.get_ws_url());
            //         }
            //     }
            // }
            Some(Ok(x)) = lines.next() => {
                if x == "help" {
                    println!("Available commands:");
                    println!("help - show this message");
                    println!("setpixel <x> <y> <r> <g> <b> - set pixel at <x>, <y> to color <r>, <g>, <b>");
                    println!("setname <name> - set your name to <name>");
                    println!("heartbeat - send a heartbeat to the server");
                    println!("requestplace - crash the program after like 3 seconds because the server sends back a fucking Vec<u8> that floods the terminal :)")
                } else {
                    let args = x.split_whitespace().map(|x| x.to_string()).collect::<Vec<String>>();
                    let thisnonce = format!("{}|{}", std::process::id(), nonce);
                    let wscommand = ToServerMsg::parse(args, thisnonce.clone());
                    match wscommand {
                        Ok(command) => {
                            ws.send(command, thisnonce).await.unwrap();
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
}
