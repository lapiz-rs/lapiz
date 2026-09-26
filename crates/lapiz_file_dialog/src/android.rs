use std::{
    ffi::c_void,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context as _, Result, anyhow, bail};
use futures::channel::oneshot;
use jni::{
    JNIEnv, JavaVM, NativeMethod, errors,
    objects::{JClass, JObject, JString, JValue},
    sys::{jint, jlong},
};
use lapiz_runtime::android::AndroidApp;

pub struct Document {
    pub uri: String,
    pub name: String,
}

type PickerResult = Result<Option<Document>>;

extern "system" fn picked(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    request: jlong,
    status: jint,
    uri: JString<'_>,
    name: JString<'_>,
) {
    if request == 0 {
        return;
    }
    // SAFETY: The request pointer was allocated by start_pick and is transferred to this callback once.
    let sender = unsafe { Box::from_raw(request as *mut oneshot::Sender<PickerResult>) };
    let result = match status {
        0 => Ok(None),
        1 => (|| -> PickerResult {
            let uri = env.get_string(&uri)?.into();
            let name = env.get_string(&name)?.into();
            Ok(Some(Document { uri, name }))
        })(),
        _ => Err(anyhow!("Android document picker failed")),
    };
    let _ = sender.send(result);
}

fn with_activity<R>(
    app: &AndroidApp,
    f: impl FnOnce(&mut JNIEnv<'_>, &JObject<'_>) -> errors::Result<R>,
) -> Result<R> {
    // SAFETY: AndroidApp retains the VM and activity references for the duration of this call.
    let vm = unsafe { JavaVM::from_raw(app.vm_as_ptr().cast()) }?;
    // SAFETY: The activity pointer is a valid JNI object reference supplied by AndroidApp.
    let activity = unsafe { JObject::from_raw(app.activity_as_ptr().cast()) };
    let mut env = vm.attach_current_thread_permanently()?;
    let result = f(&mut env, &activity);
    if env.exception_check()? {
        let exception = env.exception_occurred()?;
        env.exception_clear()?;
        let detail = env
            .call_method(exception, "toString", "()Ljava/lang/String;", &[])
            .and_then(|value| value.l())
            .and_then(|value| env.get_string(&JString::from(value)).map(Into::into));
        return Err(anyhow!(
            "Android JNI exception: {}",
            detail.unwrap_or_else(|_| String::from("unknown"))
        ));
    }
    result.map_err(Into::into)
}

pub async fn pick(app: &AndroidApp, save: bool, name: &str) -> Result<Option<Document>> {
    let receiver = start_pick(app, save, name)?;
    receiver
        .await
        .context("Document picker did not return a result")?
}

fn start_pick(app: &AndroidApp, save: bool, name: &str) -> Result<oneshot::Receiver<PickerResult>> {
    let (sender, receiver) = oneshot::channel();
    let request = Box::into_raw(Box::new(sender));
    let shown = with_activity(app, |env, activity| -> errors::Result<bool> {
        let class = env.get_object_class(activity)?;
        env.register_native_methods(
            &class,
            &[NativeMethod {
                name: "onDocumentPickedNative".into(),
                sig: "(JILjava/lang/String;Ljava/lang/String;)V".into(),
                fn_ptr: picked as *mut c_void,
            }],
        )?;
        let name = env.new_string(name)?;
        env.call_method(
            activity,
            "requestDocument",
            "(JZLjava/lang/String;)Z",
            &[
                JValue::Long(request as jlong),
                JValue::Bool(if save { 1 } else { 0 }),
                JValue::Object(name.as_ref()),
            ],
        )?
        .z()
    });
    match shown {
        Ok(true) => {}
        Ok(false) => {
            // SAFETY: The request was not accepted, so no callback will reclaim this allocation.
            unsafe { drop(Box::from_raw(request)) };
            bail!("Another document picker is already open");
        }
        Err(error) => {
            // SAFETY: The request was not accepted, so no callback will reclaim this allocation.
            unsafe { drop(Box::from_raw(request)) };
            return Err(error);
        }
    }

    Ok(receiver)
}

fn with_document(app: &AndroidApp, method: &str, uri: &str, path: &Path) -> Result<bool> {
    let path = path.to_str().context("Document path is not UTF-8")?;
    let ok = with_activity(app, |env, activity| -> errors::Result<bool> {
        let uri = env.new_string(uri)?;
        let path = env.new_string(path)?;
        env.call_method(
            activity,
            method,
            "(Ljava/lang/String;Ljava/lang/String;)Z",
            &[JValue::Object(uri.as_ref()), JValue::Object(path.as_ref())],
        )?
        .z()
    })?;
    Ok(ok)
}

pub fn temp_file(name: &str) -> Result<(PathBuf, tempfile::TempDir)> {
    let cache = lapiz_dirs::cache_dir();
    fs::create_dir_all(cache)?;
    let directory = tempfile::Builder::new()
        .prefix("document-")
        .tempdir_in(cache)?;
    let name = Path::new(name)
        .file_name()
        .filter(|name| !name.is_empty())
        .context("Document has no file name")?;
    Ok((directory.path().join(name), directory))
}

pub fn copy_file(app: &AndroidApp, uri: &str, path: &Path) -> Result<()> {
    if !with_document(app, "copyDocument", uri, path)? {
        bail!("Unable to read selected document");
    }
    Ok(())
}

pub fn write_file(app: &AndroidApp, uri: &str, path: &Path) -> Result<()> {
    if !with_document(app, "writeDocument", uri, path)? {
        bail!("Unable to write selected document");
    }
    Ok(())
}
