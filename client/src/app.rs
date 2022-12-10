use anyhow::Error;
use eframe::egui;
use eframe::epaint::Vec2;
use egui_extras::RetainedImage;
use place_rs_shared::messages::SafeInfo;
use poll_promise::Promise;

use crate::place::Place;
use crate::CONFIG;

pub struct Application {
    place: Place,
    image: RetainedImage,
    image_promise: Option<Promise<Result<RetainedImage, Error>>>,
    error: Option<String>,
    server_info: SafeInfo,
}

impl Application {
    pub async fn new() -> Result<Self, Error> {
        let place = Place::new(CONFIG.get_ws_url(), CONFIG.get_http_url()).await?;
        let image = place.get_image_async().await?;
        let info = place.get_info().await?;

        Ok(Self {
            place,
            image,
            image_promise: None,
            error: None,
            server_info: info,
        })
    }
}

impl eframe::App for Application {
    fn update(&mut self, ctx: &eframe::egui::Context, frame: &mut eframe::Frame) {
        if self.place.changed() || self.image_promise.is_some() {
            match self.image_promise.take() {
                Some(promise) => match promise.try_take() {
                    Ok(result) => {
                        match result {
                            Ok(image) => {
                                self.image = image;
                            }
                            Err(error) => {
                                self.error = Some(error.to_string());
                            }
                        }
                        self.image_promise = None;
                    }
                    Err(promise) => {
                        self.image_promise = Some(promise);
                    }
                },
                None => {
                    self.image_promise = Some(self.place.get_image_promise());
                }
            }
        }
        egui::CentralPanel::default().show(ctx, |ui| {
            if let Some(error) = self.error.take() {
                ui.label(error);
            }
            let aspect_ratio = self.server_info.size.x as f32 / self.server_info.size.y as f32;
            ui.image(self.image.texture_id(ctx), Vec2::new(ui.available_height() * aspect_ratio, ui.available_height()));
        });
        if self.image_promise.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }
}
