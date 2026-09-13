mod avif;
mod jpg;
mod lazuli;
mod pixels;
mod png;
mod simple;

pub use avif::AvifAdapter;
pub use jpg::JpgAdapter;
pub use lazuli::LazuliAdapter;
pub use png::PngAdapter;
pub use simple::{
    BmpAdapter, FarbfeldAdapter, GifAdapter, HdrAdapter, IcoAdapter, OpenExrAdapter, PnmAdapter,
    QoiAdapter, TgaAdapter, TiffAdapter, WebPAdapter,
};
