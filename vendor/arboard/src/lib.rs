#![expect(
    clippy::pub_use,
    reason = "This crate should emulate arboard if on supported platform"
)]

#[cfg(target_os = "android")]
pub use android::*;
#[cfg(not(target_os = "android"))]
pub use arboard_upstream::*;

#[cfg(target_os = "android")]
mod android {
    use std::{
        borrow::Cow,
        error, fmt,
        marker::PhantomData,
        path::{Path, PathBuf},
    };

    #[derive(Debug)]
    #[non_exhaustive]
    pub enum Error {
        ContentNotAvailable,
        ClipboardNotSupported,
        ClipboardOccupied,
        ConversionFailure,
        Unknown { description: String },
    }

    impl fmt::Display for Error {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "Clipboard is not supported on Android")
        }
    }

    impl error::Error for Error {}

    pub struct Clipboard;

    impl Clipboard {
        pub fn new() -> Result<Self, Error> {
            Err(Error::ClipboardNotSupported)
        }

        pub fn get(&mut self) -> Get<'_> {
            Get(PhantomData)
        }

        pub fn set(&mut self) -> Set<'_> {
            Set(PhantomData)
        }

        pub fn clear_with(&mut self) -> Clear<'_> {
            Clear(PhantomData)
        }

        pub fn get_text(&mut self) -> Result<String, Error> {
            Err(Error::ClipboardNotSupported)
        }

        pub fn set_text<'a, T: Into<Cow<'a, str>>>(&mut self, _text: T) -> Result<(), Error> {
            Err(Error::ClipboardNotSupported)
        }

        pub fn set_html<'a, T: Into<Cow<'a, str>>>(
            &mut self,
            _html: T,
            _alt_text: Option<T>,
        ) -> Result<(), Error> {
            Err(Error::ClipboardNotSupported)
        }

        pub fn clear(&mut self) -> Result<(), Error> {
            Err(Error::ClipboardNotSupported)
        }

        #[cfg(feature = "image-data")]
        pub fn get_image(&mut self) -> Result<ImageData<'static>, Error> {
            Err(Error::ClipboardNotSupported)
        }

        #[cfg(feature = "image-data")]
        pub fn set_image(&mut self, _image: ImageData<'_>) -> Result<(), Error> {
            Err(Error::ClipboardNotSupported)
        }
    }

    pub struct Get<'a>(PhantomData<&'a mut Clipboard>);

    impl Get<'_> {
        pub fn text(self) -> Result<String, Error> {
            Err(Error::ClipboardNotSupported)
        }

        pub fn html(self) -> Result<String, Error> {
            Err(Error::ClipboardNotSupported)
        }

        pub fn file_list(self) -> Result<Vec<PathBuf>, Error> {
            Err(Error::ClipboardNotSupported)
        }

        #[cfg(feature = "image-data")]
        pub fn image(self) -> Result<ImageData<'static>, Error> {
            Err(Error::ClipboardNotSupported)
        }
    }

    pub struct Set<'a>(PhantomData<&'a mut Clipboard>);

    impl Set<'_> {
        pub fn text<'a, T: Into<Cow<'a, str>>>(self, _text: T) -> Result<(), Error> {
            Err(Error::ClipboardNotSupported)
        }

        pub fn html<'a, T: Into<Cow<'a, str>>>(
            self,
            _html: T,
            _alt_text: Option<T>,
        ) -> Result<(), Error> {
            Err(Error::ClipboardNotSupported)
        }

        pub fn file_list(self, _files: &[impl AsRef<Path>]) -> Result<(), Error> {
            Err(Error::ClipboardNotSupported)
        }

        #[cfg(feature = "image-data")]
        pub fn image(self, _image: ImageData<'_>) -> Result<(), Error> {
            Err(Error::ClipboardNotSupported)
        }
    }

    pub struct Clear<'a>(PhantomData<&'a mut Clipboard>);

    impl Clear<'_> {
        pub fn default(self) -> Result<(), Error> {
            Err(Error::ClipboardNotSupported)
        }
    }

    #[cfg(feature = "image-data")]
    #[derive(Clone, Debug)]
    pub struct ImageData<'a> {
        pub width: usize,
        pub height: usize,
        pub bytes: Cow<'a, [u8]>,
    }
}
