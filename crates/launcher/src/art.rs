//! Async art loading: worker threads decode PNGs off the render thread,
//! the GL thread only uploads finished pixels. Panels that aren't decoded
//! yet draw as placeholders and pop in a frame later — scrolling never
//! waits on a decode.

use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

pub enum Art {
    Pending,
    Ready(kui_gfx::Texture),
    Missing,
}

type DecodeResult = Result<(u32, u32, Vec<u8>), String>;

pub struct Loader {
    tx: mpsc::Sender<(u32, PathBuf)>,
    rx: mpsc::Receiver<(u32, DecodeResult)>,
}

impl Loader {
    pub fn new(workers: usize) -> Self {
        let (tx, req_rx) = mpsc::channel::<(u32, PathBuf)>();
        let (res_tx, rx) = mpsc::channel();
        let req_rx = Arc::new(Mutex::new(req_rx));
        for _ in 0..workers {
            let req_rx = Arc::clone(&req_rx);
            let res_tx = res_tx.clone();
            std::thread::spawn(move || {
                loop {
                    let job = req_rx.lock().unwrap().recv();
                    let Ok((key, path)) = job else { return };
                    let _ = res_tx.send((key, kui_gfx::decode_png(&path)));
                }
            });
        }
        Self { tx, rx }
    }

    pub fn request(&self, key: u32, path: PathBuf) {
        let _ = self.tx.send((key, path));
    }

    pub fn try_recv(&self) -> Option<(u32, DecodeResult)> {
        self.rx.try_recv().ok()
    }
}

pub fn key(kind: u32, index: usize) -> u32 {
    (kind << 16) | index as u32
}

pub fn split(key: u32) -> (u32, usize) {
    (key >> 16, (key & 0xFFFF) as usize)
}
