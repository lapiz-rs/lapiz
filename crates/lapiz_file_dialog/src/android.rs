use std::{ffi::c_void, fs::File, os::fd::FromRawFd as _};

use anyhow::{Context as _, Result, anyhow, bail};
use futures::channel::oneshot;
use jni::{
    JNIEnv, JavaVM, NativeMethod, errors,
    objects::{JClass, JObject, JObjectArray, JString, JValue},
    sys::{jint, jlong, jsize},
};
use lapiz_runtime::android::AndroidApp;

/// File format filter passed to the Android document picker when saving.
pub struct FileFilter {
    pub name: String,
    pub extensions: Vec<String>,
}

/// Metadata returned by the Android document picker, containing the content URI and display name.
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

fn new_string_array<'a>(
    env: &mut JNIEnv<'a>,
    items: &[String],
) -> errors::Result<JObjectArray<'a>> {
    let string_class = env.find_class("java/lang/String")?;
    let array = env.new_object_array(items.len() as jsize, string_class, JObject::null())?;
    for (index, item) in items.iter().enumerate() {
        let value = env.new_string(item)?;
        env.set_object_array_element(&array, index as jsize, value)?;
    }
    Ok(array)
}

pub async fn pick(
    app: &AndroidApp,
    save: bool,
    name: &str,
    filters: &[FileFilter],
) -> Result<Option<Document>> {
    let receiver = start_pick(app, save, name, filters)?;
    receiver
        .await
        .context("Document picker did not return a result")?
}

fn start_pick(
    app: &AndroidApp,
    save: bool,
    name: &str,
    filters: &[FileFilter],
) -> Result<oneshot::Receiver<PickerResult>> {
    let filter_labels = filters
        .iter()
        .map(|filter| {
            let exts = filter
                .extensions
                .iter()
                .map(|ext| format!(".{ext}"))
                .collect::<Vec<_>>()
                .join(", ");
            format!("{} ({exts})", filter.name)
        })
        .collect::<Vec<_>>();
    let filter_extensions = filters
        .iter()
        .map(|filter| filter.extensions.join(","))
        .collect::<Vec<_>>();

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
        let labels = new_string_array(env, &filter_labels)?;
        let extensions = new_string_array(env, &filter_extensions)?;
        env.call_method(
            activity,
            "requestDocument",
            "(JZLjava/lang/String;[Ljava/lang/String;[Ljava/lang/String;)Z",
            &[
                JValue::Long(request as jlong),
                JValue::Bool(if save { 1 } else { 0 }),
                JValue::Object(name.as_ref()),
                JValue::Object(labels.as_ref()),
                JValue::Object(extensions.as_ref()),
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

pub fn open_file(app: &AndroidApp, uri: &str, mode: &str) -> Result<File> {
    let fd = with_activity(app, |env, activity| -> errors::Result<jint> {
        let uri = env.new_string(uri)?;
        let mode = env.new_string(mode)?;
        env.call_method(
            activity,
            "openDocumentFd",
            "(Ljava/lang/String;Ljava/lang/String;)I",
            &[JValue::Object(uri.as_ref()), JValue::Object(mode.as_ref())],
        )?
        .i()
    })?;
    if fd < 0 {
        bail!("Unable to open document file descriptor");
    }
    // SAFETY: `detachFd` transfers sole ownership of the open file descriptor to Rust.
    let file = unsafe { File::from_raw_fd(fd) };
    Ok(file)
}
