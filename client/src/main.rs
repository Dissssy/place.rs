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
                println!("Received: {:?}", m);
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
