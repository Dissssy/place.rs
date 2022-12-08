#![allow(clippy::await_holding_lock)]
use actix::fut::wrap_future;
use actix::ActorContext;
use actix::AsyncContext;
use actix::StreamHandler;
use actix_web::{web, App, HttpRequest, HttpResponse, HttpServer};
use actix_web_actors::ws;
use actix_web_actors::ws::CloseCode;
use actix_web_actors::ws::CloseReason;
use anyhow::Error;
use config::Timeouts;
use interfaces::PostgresConfig;
use lazy_static::lazy_static;
use place_rs_shared::messages::TimeoutType;
use place_rs_shared::messages::ToServerMsg;
use sha2::Digest;
use sha2::Sha256;
use std::sync::Arc;
use tokio::sync::mpsc::error::TryRecvError;
mod config;
mod interfaces;
use config::Config;
use place_rs_shared::{messages::ToClientMsg, program_path, MetaPlace, User};
use std::sync::Mutex;
use tokio::sync::mpsc::UnboundedReceiver;

lazy_static! {
    static ref CONFIG: Config = Config::new().unwrap();
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    let place = get_server_place().await.unwrap();
    println!("Starting server on {}:{}", CONFIG.host, CONFIG.port);
    let place_clone = place.clone();
    /*let x = */
    HttpServer::new(move || App::new().app_data(web::Data::new(place_clone.clone())).route("/ws/", web::get().to(ws)))
        .bind((CONFIG.host.clone(), CONFIG.port))?
        .run()
        .await
        .expect("Failed to start server");

    place.lock().unwrap().save().await.unwrap();
    Ok(())
}

async fn get_server_place() -> Result<Arc<Mutex<MetaPlace>>, Error> {
    match &CONFIG.interface {
        interfaces::Interface::Json => get_json_place().await,
        interfaces::Interface::Postgres(config) => get_postgres_place(config.clone()).await,
        interfaces::Interface::Gzip => get_gzip_place().await,
    }
}

async fn get_json_place() -> Result<Arc<Mutex<MetaPlace>>, Error> {
    let mut path = program_path()?;
    path.push("place.json");
    Ok(Arc::new(Mutex::new(MetaPlace::new(Box::new(interfaces::JsonInterface::new(path))).await?)))
}

async fn get_postgres_place(_config: PostgresConfig) -> Result<Arc<Mutex<MetaPlace>>, Error> {
    // Ok(Arc::new(Mutex::new(MetaPlace::new(Box::new(interfaces::PostgresInterface::new(config).await?)).await?)))
    todo!()
}

async fn get_gzip_place() -> Result<Arc<Mutex<MetaPlace>>, Error> {
    let mut path = program_path()?;
    path.push("place.gz");
    Ok(Arc::new(Mutex::new(MetaPlace::new(Box::new(interfaces::GzipInterface::new(path))).await?)))
}

async fn ws(req: HttpRequest, stream: web::Payload, data: web::Data<Arc<Mutex<MetaPlace>>>) -> Result<HttpResponse, actix_web::Error> {
    let place = data.get_ref().clone();
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let id = hash(req.peer_addr().unwrap().ip().to_string());
    let user = place.lock().unwrap().add_websocket(id.clone(), tx);
    let myws = WsConnection {
        place,
        rx: Some(rx),
        user,
        id,
        sent_heartbeats: 0,
        timeouts: Timeouts::default(),
        place_requested: false,
    };
    ws::start(myws, &req, stream)
}

fn hash(s: String) -> String {
    let mut hasher = Sha256::new();
    hasher.update(s);
    let result = hasher.finalize();
    format!("{:x}", result)
}

struct WsConnection {
    place: Arc<Mutex<MetaPlace>>,
    rx: Option<UnboundedReceiver<ToClientMsg>>,
    id: String,
    user: Option<User>,
    sent_heartbeats: u32,
    timeouts: Timeouts,
    place_requested: bool,
}

impl StreamHandler<Result<ws::Message, ws::ProtocolError>> for WsConnection {
    fn handle(&mut self, msg: Result<ws::Message, ws::ProtocolError>, ctx: &mut Self::Context) {
        let now = chrono::Utc::now().timestamp() as u64;
        match msg {
            Ok(ws::Message::Text(text)) => {
                let msg = serde_json::from_str::<ToServerMsg>(&text);
                match msg {
                    Ok(msg) => match msg {
                        ToServerMsg::Heartbeat => {
                            self.sent_heartbeats = 0;
                        }
                        ToServerMsg::SetName(name) => {
                            if now < self.timeouts.username {
                                ctx.text(serde_json::to_string(&ToClientMsg::TimeoutError(TimeoutType::Username(CONFIG.timeouts.username))).unwrap());
                            } else {
                                self.timeouts.username = now + CONFIG.timeouts.username;
                                if let Some(user) = &self.user {
                                    let r = self.place.lock().unwrap().update_username(&user.id, &name);
                                    match r {
                                        Ok(_) => {}
                                        Err(e) => {
                                            ctx.text(serde_json::to_string(&ToClientMsg::GenericError(e.to_string())).unwrap());
                                        }
                                    }
                                } else {
                                    let user = User { id: self.id.clone(), name };
                                    let r = self.place.lock().unwrap().add_user(user.clone());
                                    match r {
                                        Ok(_) => {
                                            self.user = Some(user);
                                        }
                                        Err(e) => {
                                            ctx.text(serde_json::to_string(&ToClientMsg::GenericError(e.to_string())).unwrap());
                                        }
                                    }
                                }
                            }
                        }
                        ToServerMsg::SetPixel(pixel) => {
                            if now < self.timeouts.paint {
                                ctx.text(serde_json::to_string(&ToClientMsg::TimeoutError(TimeoutType::Pixel(CONFIG.timeouts.paint))).unwrap());
                            } else if self.user.is_some() {
                                self.timeouts.paint = now + CONFIG.timeouts.paint;
                                let r = self.place.lock().unwrap().update_pixel(&pixel.into_full(self.id.clone()));
                                match r {
                                    Ok(_) => {}
                                    Err(e) => {
                                        ctx.text(serde_json::to_string(&ToClientMsg::GenericError(e.to_string())).unwrap());
                                    }
                                }
                            } else {
                                ctx.text(serde_json::to_string(&ToClientMsg::GenericError("You must set a username before painting".to_string())).unwrap());
                            }
                        }
                        ToServerMsg::RequestPlace => {
                            // client is requesting a gzipped place, this operation is very expensive so we only allow it once per connection
                            if !self.place_requested {
                                let place = self.place.lock().unwrap();
                                let p = place.place.clone();
                                ctx.run_later(std::time::Duration::from_millis(0), move |act, ctx| {
                                    let bin = p.gun_zip();
                                    match bin {
                                        Ok(bin) => {
                                            act.place_requested = true;
                                            ctx.binary(bin)
                                        }
                                        Err(e) => {
                                            ctx.text(serde_json::to_string(&ToClientMsg::GenericError(e.to_string())).unwrap());
                                        }
                                    }
                                });
                            } else {
                                ctx.text(serde_json::to_string(&ToClientMsg::GenericError("You have already requested the place".to_string())).unwrap());
                            }
                        }
                    },
                    Err(e) => ctx.text(serde_json::to_string(&ToClientMsg::GenericError(e.to_string())).unwrap()),
                }
            }
            Ok(ws::Message::Close(reason)) => {
                self.place.lock().unwrap().remove_websocket(&self.id);
                // println!("Closing websocket: {:?}", reason);
                ctx.close(reason);
                ctx.stop();
            }
            Ok(ws::Message::Ping(msg)) => {
                ctx.pong(&msg);
            }
            Ok(ws::Message::Pong(_)) => {
                ctx.text(serde_json::to_string(&ToClientMsg::GenericError("Unsupported method Pong".to_string())).unwrap());
            }
            Ok(ws::Message::Binary(_)) => {
                ctx.text(serde_json::to_string(&ToClientMsg::GenericError("Unsupported method Binary".to_string())).unwrap());
            }
            Ok(ws::Message::Nop) => {
                ctx.text(serde_json::to_string(&ToClientMsg::GenericError("Unsupported method Nop".to_string())).unwrap());
            }
            Ok(ws::Message::Continuation(_)) => {
                ctx.text(serde_json::to_string(&ToClientMsg::GenericError("Unsupported method Continuation".to_string())).unwrap());
            }
            Err(e) => {
                ctx.text(serde_json::to_string(&ToClientMsg::GenericError(e.to_string())).unwrap());
            }
        }
    }

    fn started(&mut self, ctx: &mut Self::Context) {
        // println!("Websocket connection started for {}", self.id);
        let mut rx = self.rx.take().unwrap();
        ctx.run_interval(std::time::Duration::from_millis(CONFIG.times.ws_msg_interval), move |_act, ctx| {
            let x = rx.try_recv();
            match x {
                Ok(msg) => ctx.text(serde_json::to_string(&msg).unwrap()),
                Err(err) => match err {
                    TryRecvError::Empty => {}
                    TryRecvError::Disconnected => {
                        // println!("Message channel disconnected");
                        ctx.close(Some(CloseReason {
                            code: CloseCode::Error,
                            description: Some(serde_json::to_string(&ToClientMsg::GenericError("Disconnected".to_string())).unwrap()),
                        }));
                    }
                },
            }
        });
        ctx.run_interval(std::time::Duration::from_millis(CONFIG.times.ws_hb_interval), move |act, ctx| {
            if act.sent_heartbeats < CONFIG.max_missed_heartbeats {
                ctx.text(serde_json::to_string(&ToClientMsg::Heartbeat).unwrap());
                act.sent_heartbeats += 1;
            } else {
                // println!("Heartbeat timeout");
                ctx.close(Some(CloseReason {
                    code: CloseCode::Error,
                    description: Some("Heartbeat timeout".to_string()),
                }));
            }
        });
    }
}

impl actix::Actor for WsConnection {
    type Context = ws::WebsocketContext<Self>;
}
