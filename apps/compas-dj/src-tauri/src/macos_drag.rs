//! macOS claims every drag when file drops are enabled, which stops row reordering.
//! File drags stay with that handler. Drags that are not files go back to the webview.

use std::ffi::CStr;
use std::mem::{size_of, transmute_copy};
use std::sync::OnceLock;

use objc2::msg_send;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Imp, Sel};
use objc2_app_kit::NSPasteboard;
use objc2_foundation::NSArray;

type EnteredFn = unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject) -> usize;
#[cfg(target_arch = "aarch64")]
type DragBool = bool;
#[cfg(not(target_arch = "aarch64"))]
type DragBool = i8;
type PerformFn = unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject) -> DragBool;
type ExitedFn = unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject);

struct DragImps {
    entered_files: EnteredFn,
    entered_webview: EnteredFn,
    updated_files: EnteredFn,
    updated_webview: EnteredFn,
    perform_files: PerformFn,
    perform_webview: PerformFn,
    exited_files: ExitedFn,
    exited_webview: ExitedFn,
}

static IMPS: OnceLock<DragImps> = OnceLock::new();

pub fn keep_html_drags_with_the_webview() {
    let Some(class) = wry_webview_class() else {
        tracing::warn!("internal row drags could not be restored");
        return;
    };
    let Some(entered) = method_pair(class, c"draggingEntered:") else {
        tracing::warn!("internal row drags could not be restored");
        return;
    };
    let Some(updated) = method_pair(class, c"draggingUpdated:") else {
        tracing::warn!("internal row drags could not be restored");
        return;
    };
    let Some(perform) = method_pair(class, c"performDragOperation:") else {
        tracing::warn!("internal row drags could not be restored");
        return;
    };
    let Some(exited) = method_pair(class, c"draggingExited:") else {
        tracing::warn!("internal row drags could not be restored");
        return;
    };
    if IMPS
        .set(DragImps {
            entered_files: as_fn(entered.files),
            entered_webview: as_fn(entered.webview),
            updated_files: as_fn(updated.files),
            updated_webview: as_fn(updated.webview),
            perform_files: as_fn(perform.files),
            perform_webview: as_fn(perform.webview),
            exited_files: as_fn(exited.files),
            exited_webview: as_fn(exited.webview),
        })
        .is_err()
    {
        return;
    }
    let entered_imp: EnteredFn = dragging_entered;
    let updated_imp: EnteredFn = dragging_updated;
    let perform_imp: PerformFn = perform_drag_operation;
    let exited_imp: ExitedFn = dragging_exited;
    // SAFETY: each replacement has the same signature as the method it overrides,
    // and the previous implementation is kept for file drags.
    unsafe {
        entered.method.set_implementation(as_imp(entered_imp));
        updated.method.set_implementation(as_imp(updated_imp));
        perform.method.set_implementation(as_imp(perform_imp));
        exited.method.set_implementation(as_imp(exited_imp));
    }
}

struct MethodPair<'a> {
    method: &'a objc2::runtime::Method,
    files: Imp,
    webview: Imp,
}

fn wry_webview_class() -> Option<&'static AnyClass> {
    AnyClass::classes().iter().copied().find(|class| {
        let name = class.name().to_string_lossy();
        let Some(leaf) = name.rsplit("::").next() else {
            return false;
        };
        leaf.strip_prefix("WryWebView")
            .is_some_and(|rest| rest.starts_with(|character: char| character.is_ascii_digit()))
    })
}

fn method_pair<'a>(class: &'a AnyClass, name: &CStr) -> Option<MethodPair<'a>> {
    let selector = Sel::register(name);
    let method = class.instance_method(selector)?;
    let superclass = class.superclass()?;
    let webview = superclass.instance_method(selector)?;
    let files = method.implementation();
    let webview_imp = webview.implementation();
    if imp_address(files) == imp_address(webview_imp) {
        return None;
    }
    Some(MethodPair {
        method,
        files,
        webview: webview_imp,
    })
}

fn drag_has_files(info: *mut AnyObject) -> bool {
    if info.is_null() {
        return false;
    }
    let info = unsafe { &*info };
    // SAFETY: AppKit passes the current drag info, which responds to draggingPasteboard.
    let pasteboard: Option<Retained<NSPasteboard>> = unsafe { msg_send![info, draggingPasteboard] };
    let Some(pasteboard) = pasteboard else {
        return false;
    };
    let types = NSArray::arrayWithObject(filenames_type());
    pasteboard.availableTypeFromArray(&types).is_some()
}

#[allow(deprecated)]
fn filenames_type() -> &'static objc2_foundation::NSString {
    // SAFETY: AppKit defines this pasteboard type for the life of the process.
    unsafe { objc2_app_kit::NSFilenamesPboardType }
}

unsafe extern "C-unwind" fn dragging_entered(
    this: *mut AnyObject,
    cmd: Sel,
    info: *mut AnyObject,
) -> usize {
    let Some(imps) = IMPS.get() else {
        return 0;
    };
    let call = if drag_has_files(info) {
        imps.entered_files
    } else {
        imps.entered_webview
    };
    // SAFETY: the stored implementation is the method that was installed for this selector.
    unsafe { call(this, cmd, info) }
}

unsafe extern "C-unwind" fn dragging_updated(
    this: *mut AnyObject,
    cmd: Sel,
    info: *mut AnyObject,
) -> usize {
    let Some(imps) = IMPS.get() else {
        return 0;
    };
    let call = if drag_has_files(info) {
        imps.updated_files
    } else {
        imps.updated_webview
    };
    // SAFETY: the stored implementation is the method that was installed for this selector.
    unsafe { call(this, cmd, info) }
}

unsafe extern "C-unwind" fn perform_drag_operation(
    this: *mut AnyObject,
    cmd: Sel,
    info: *mut AnyObject,
) -> DragBool {
    let Some(imps) = IMPS.get() else {
        return drag_bool(false);
    };
    let call = if drag_has_files(info) {
        imps.perform_files
    } else {
        imps.perform_webview
    };
    // SAFETY: the stored implementation is the method that was installed for this selector.
    unsafe { call(this, cmd, info) }
}

unsafe extern "C-unwind" fn dragging_exited(this: *mut AnyObject, cmd: Sel, info: *mut AnyObject) {
    let Some(imps) = IMPS.get() else {
        return;
    };
    let call = if drag_has_files(info) {
        imps.exited_files
    } else {
        imps.exited_webview
    };
    // SAFETY: the stored implementation is the method that was installed for this selector.
    unsafe { call(this, cmd, info) }
}

fn drag_bool(value: bool) -> DragBool {
    #[cfg(target_arch = "aarch64")]
    {
        value
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        if value {
            1
        } else {
            0
        }
    }
}

fn imp_address(imp: Imp) -> usize {
    imp as usize
}

fn as_fn<T>(imp: Imp) -> T {
    const {
        assert!(size_of::<T>() == size_of::<Imp>());
    }
    // SAFETY: T is the function signature of this Objective-C method implementation.
    unsafe { transmute_copy(&imp) }
}

fn as_imp<T>(function: T) -> Imp {
    const {
        assert!(size_of::<T>() == size_of::<Imp>());
    }
    // SAFETY: T is an extern C-unwind function with the method's calling convention.
    unsafe { transmute_copy(&function) }
}
