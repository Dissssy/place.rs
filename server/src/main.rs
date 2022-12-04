use actix::fut::wrap_future;
use actix::prelude::*;
use actix_web::dev::Path;
use actix_web::dev::ServerHandle;
use actix_web_actors::ws;
use place_rs_shared::SafeInfo;
use serde::{Deserialize, Serialize};
use sha2::Digest;
use sha2::Sha256;
// use sqlx::postgres::PgPoolOptions;
use std::sync::mpsc;
use std::sync::Arc;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::oneshot;
use tokio::sync::Mutex;

use actix_web::{get, web, App, HttpServer, Responder};
use place_rs_shared::{Color, Pixel, Place, RawWebsocketMessage, ServerConfig, User, XY};

#[get("/api/canvas/{x}/{y}")]
async fn api_canvas(data: web::Data<Arc<Mutex<Place>>>, coord: web::Path<(usize, usize)>) -> impl Responder {
    // serve the current state of the canvas as json
    // we will take advantage of chunking to make this a faster process, x and y are the 128x128 chunk coordinates
    let data = data.lock().await;
    let chunk = data.get_chunk(coord.0, coord.1);
    web::Json(chunk)
}

#[get("/api/info")]
async fn api_info() -> impl Responder {
    let config = ServerConfig::load().unwrap();
    let info = SafeInfo {
        timeout: config.timeout,
        size: config.size,
        chunk_size: config.chunk_size,
    };
    web::Json(info)
}

// an api endpoint to restart the server, debugging purposes
#[get("/api/restart")]
async fn stop(stop_handle: web::Data<StopHandle>) -> HttpResponse {
    stop_handle.stop(true);
    HttpResponse::NoContent().finish()
}

#[actix_web::main] // or #[tokio::main]
async fn main() -> std::io::Result<()> {
    let config = ServerConfig::load().unwrap();
    // we will run a websocket server on the same port as the http server under the /ws path, this will serve live events and updates to the client, specifically when a pixel changes, and the current timeout for the user. the user will push messages to this websocket to update a pixel
    // we will run a http server on the same port as the websocket server under the /api path, this will serve the basic api endpoints to get the current state of the canvas, to get the user up to date
    let place = Arc::new(Mutex::new(Place::load().await.unwrap()));
    let stop_handle = web::Data::new(StopHandle::default());
    let sandle = stop_handle.clone();
    let place_clone = place.clone();
    println!("Starting server on port {}", config.port);
    let x = HttpServer::new(move || {
        let stop_handle = stop_handle.clone();
        App::new()
            .app_data(web::Data::new(place_clone.clone()))
            .app_data(stop_handle)
            .service(api_canvas)
            .service(api_info)
            .service(stop)
            .route("/ws/", web::get().to(ws_index))
    })
    .bind((config.host, config.port))?
    .run();
    sandle.register(x.handle());
    let x = x.await;

    println!("{:?}", x);
    println!("Shutting down server");
    println!("Saving canvas");
    let place = place.lock().await;
    println!("{:?}", place.handler.save(place.get_save_data()).await);

    x
}

#[derive(Default)]
struct StopHandle {
    inner: std::sync::Mutex<Option<ServerHandle>>,
}

impl StopHandle {
    /// Sets the server handle to stop.
    pub(crate) fn register(&self, handle: ServerHandle) {
        let mut binding = self.inner.lock();
        let t = binding.as_deref_mut().unwrap();
        let mut handle = Some(handle);
        std::mem::swap(t, &mut handle);
    }

    /// Sends stop signal through contained server handle.
    pub(crate) fn stop(&self, graceful: bool) {
        let _ = self.inner.lock().unwrap().take().unwrap().stop(graceful);
    }
}

use actix::{Actor, StreamHandler};
use actix_web::{HttpRequest, HttpResponse};

/// Define HTTP actor
struct MyWs {
    place: Arc<Mutex<Place>>,
    rx: Option<UnboundedReceiver<WebsocketMessage>>,
    id: String,
    user: Option<User>,
    lasthb: std::time::Instant,
    heartbeats: i64,
    timeout: i64,
}

impl MyWs {
    pub fn heartbeat(&mut self) -> bool {
        let now = std::time::Instant::now();
        if now.duration_since(self.lasthb) > std::time::Duration::from_secs(5) {
            self.lasthb = now;
            self.heartbeats += 1;
            return true;
        }
        false
    }
}

impl Actor for MyWs {
    // we will need two way async communication with the websocket, so we will use an unbounded channel
    type Context = ws::WebsocketContext<Self>;
}

// we need to be able to handle messages FROM the websocket, and push messages TO the websocket
// we will use the WebSocketMessage enum from the shared crate to handle this
use place_rs_shared::WebsocketMessage;

async fn ws_index(req: HttpRequest, stream: web::Payload, data: web::Data<Arc<Mutex<Place>>>) -> Result<HttpResponse, actix_web::Error> {
    let place = data.get_ref().clone();
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let id = hash(req.peer_addr().unwrap().ip().to_string());
    let user = place.lock().await.add_websocket(id.clone(), tx);
    let config = ServerConfig::load().unwrap();
    let myws = MyWs {
        place,
        rx: Some(rx),
        user,
        id,
        lasthb: std::time::Instant::now(),
        heartbeats: 0,
        timeout: config.username_timeout,
    };
    ws::start(myws, &req, stream)
}

// listen for messages from the websocket and handle them, while using heartbeat to keep the connection alive
// #[async_trait::async_trait]
impl StreamHandler<Result<ws::Message, ws::ProtocolError>> for MyWs {
    fn handle(&mut self, msg: Result<ws::Message, ws::ProtocolError>, ctx: &mut Self::Context) {
        match msg {
            Ok(ws::Message::Ping(msg)) => ctx.pong(&msg),
            Ok(ws::Message::Text(text)) => {
                // let th = RawWebsocketMessage::PixelUpdate {
                //     x: 0,
                //     y: 0,
                //     color: Color { r: 0, g: 0, b: 0 },
                // };
                // println!("json: {}", serde_json::to_string(&th).unwrap());
                // we will use the WebsocketMessage enum from the shared crate to handle this
                let msg = serde_json::from_str::<RawWebsocketMessage>(&text);
                if let Ok(msg) = msg {
                    match msg {
                        RawWebsocketMessage::PixelUpdate { location, color } => {
                            if self.rx.is_some() {
                                let err = serde_json::to_string(&WebsocketMessage::Error("Initialise the listener first".to_string())).unwrap();
                                ctx.text(err);
                                return;
                            }
                            let user = self.user.clone();
                            if let Some(user) = user {
                                let time = chrono::Utc::now().timestamp();
                                let pixel = Pixel {
                                    location,
                                    color,
                                    timestamp: Some(time),
                                    user: Some(user.id),
                                };
                                let (tx, mut rx) = oneshot::channel::<String>();
                                let f = wrap_future(update_pixel(pixel, self.place.clone(), tx));
                                ctx.spawn(f);
                                ctx.run_interval(std::time::Duration::from_millis(100), move |_act, ctx| {
                                    if let Ok(msg) = rx.try_recv() {
                                        let err = serde_json::to_string(&WebsocketMessage::Error(msg)).unwrap();
                                        ctx.text(err);
                                    }
                                });
                            } else {
                                let err = serde_json::to_string(&WebsocketMessage::Error("Set a nickname first".to_string())).unwrap();
                                ctx.text(err);
                            }
                        }
                        RawWebsocketMessage::Heartbeat => {
                            if self.rx.is_some() {
                                let err = serde_json::to_string(&WebsocketMessage::Error("Initialise the listener first".to_string())).unwrap();
                                ctx.text(err);
                                return;
                            }
                            // send the heartbeat to the websocket
                            self.heartbeats = 0;
                        }
                        RawWebsocketMessage::Listen => {
                            // check if the handle is already set, if it is, return an error message

                            let rx = self.rx.take();
                            if let Some(mut rx) = rx {
                                ctx.run_interval(std::time::Duration::from_millis(100), move |act, ctx| {
                                    // check if the websocket is still connected
                                    if !ctx.state().alive() {
                                        // if it is not, remove the websocket from the place
                                        // act.place.lock().await.remove_websocket(hash(act.id.clone()));
                                        ctx.stop();
                                        return;
                                    }
                                    // check if there is a message in the channel
                                    if let Ok(msg) = rx.try_recv() {
                                        // if there is, send it to the websocket
                                        ctx.text(serde_json::to_string(&msg).unwrap());
                                    }
                                    if act.heartbeat() {
                                        if act.heartbeats > 5 {
                                            ctx.stop();
                                        } else {
                                            ctx.text(serde_json::to_string(&WebsocketMessage::Heartbeat).unwrap());
                                        }
                                    }
                                });
                                let listeningmsg = WebsocketMessage::Listening;
                                ctx.text(serde_json::to_string(&listeningmsg).unwrap());
                                // let (mtx, mut mrx) = tokio::sync::mpsc::unbounded_channel::<String>();
                                // let (ktx, mut krx) = tokio::sync::oneshot::channel::<()>();
                                // let _: Promise<()> = Promise::spawn_thread(format!("listener:{}", self.ip), move || {
                                //     loop {
                                //         if let Ok(()) = krx.try_recv() {
                                //             break;
                                //         }
                                //         // check if there is a message in the channel
                                //         if let Ok(msg) = rx.try_recv() {
                                //             // send the message to the websocket
                                //             mtx.send(serde_json::to_string(&msg).unwrap()).unwrap();
                                //         }
                                //         // sleep for 100ms
                                //         std::thread::sleep();
                                //     }
                                // });
                            } else {
                                let err = serde_json::to_string(&WebsocketMessage::Error("Already listening".to_string())).unwrap();
                                ctx.text(err);
                            }
                        }
                        RawWebsocketMessage::SetUsername(username) => {
                            if self.rx.is_some() {
                                let err = serde_json::to_string(&WebsocketMessage::Error("Initialise the listener first".to_string())).unwrap();
                                ctx.text(err);
                                return;
                            }
                            let now = chrono::Utc::now().timestamp();
                            // if the users timeout is less than the current time then error
                            if let Some(user) = self.user.as_mut() {
                                if user.username_timeout > now {
                                    let err = serde_json::to_string(&WebsocketMessage::Error("You are timed out".to_string())).unwrap();
                                    ctx.text(err);
                                    return;
                                }
                                let (tx, mut rx) = oneshot::channel::<String>();
                                user.name = username;
                                user.username_timeout = now + self.timeout;
                                let f = wrap_future(set_username(user.clone(), self.place.clone(), tx));
                                ctx.spawn(f);
                                ctx.run_interval(std::time::Duration::from_millis(100), move |_act, ctx| {
                                    if let Ok(msg) = rx.try_recv() {
                                        ctx.text(msg);
                                    }
                                });
                            } else {
                                self.user = Some(User {
                                    name: username,
                                    id: self.id.clone(),
                                    username_timeout: now + self.timeout,
                                    timeout: 0,
                                });
                            }
                            // println!("{:?}", self.user);
                        }
                        _ => {
                            ctx.text(
                                serde_json::to_string(&RawWebsocketMessage::Error {
                                    message: "Unknown message".to_string(),
                                })
                                .unwrap(),
                            );
                        }
                    }
                } else {
                    ctx.text(
                        serde_json::to_string(&RawWebsocketMessage::Error {
                            message: "Invalid message".to_string(),
                        })
                        .unwrap(),
                    );
                }
            }
            Ok(ws::Message::Close(reason)) => {
                // remove the websocket from the place
                // let mut place = self.place.lock().await;
                // place.remove_websocket(ctx.address());
                ctx.close(reason);
                ctx.stop();
            }
            _ => ctx.stop(),
        }
    }
}

fn hash(s: String) -> String {
    let mut hasher = Sha256::new();
    hasher.update(s);
    let result = hasher.finalize();
    format!("{:x}", result)
}

async fn update_pixel(pixel: Pixel, place: Arc<Mutex<Place>>, tx: oneshot::Sender<String>) {
    let mut place = place.lock().await;
    let r = place.set_pixel(pixel).await;
    if let Err(r) = r {
        tx.send(r.to_string()).unwrap();
    }
}

async fn set_username(user: User, place: Arc<Mutex<Place>>, tx: oneshot::Sender<String>) {
    let mut place = place.lock().await;
    let r = place.set_username(user.id, user.name).await;
    if let Err(r) = r {
        tx.send(r.to_string()).unwrap();
    }
}
