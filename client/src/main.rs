use async_std::io::prelude::*;
use async_std::io::stdin;
use async_std::io::BufReader;
use async_std::stream::StreamExt;
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
    loop {
        tokio::select! {
            m = ws.recv() => {
                match m {
                    Some(msg) => {
                        println!("Received: {:?}", msg);
                    }
                    None => {
                        println!("Disconnected");
                        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                        ws = websocket::Websocket::new(CONFIG.get_ws_url());
                    }
                }
            }
            Some(Ok(x)) = lines.next() => {
                let args = x.split_whitespace().map(|x| x.to_string()).collect::<Vec<String>>();
                let wscommand = ToServerMsg::parse(args);
                match wscommand {
                    Ok(command) => {
                        ws.send(command).await.unwrap();
                    }
                    Err(e) => {
                        println!("Error: {}", e);
                    }
                }
            }
        }
    }
}
