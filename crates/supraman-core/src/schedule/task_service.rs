use windows::Win32::Foundation::{CloseHandle, HANDLE, HLOCAL, LocalFree};
use windows::Win32::Security::Authorization::ConvertSidToStringSidW;
use windows::Win32::Security::{GetTokenInformation, TOKEN_QUERY, TOKEN_USER, TokenUser};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize};
use windows::Win32::System::TaskScheduler::{IRegisteredTask, ITaskFolder, ITaskService, TASK_CREATE_OR_UPDATE, TASK_ENUM_HIDDEN, TASK_LOGON_INTERACTIVE_TOKEN, TaskScheduler};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::Win32::System::Variant::VARIANT;
use windows::core::{BSTR, HRESULT, PWSTR};

use super::foreign::ole_date_ms;
use super::registrar::{Listing, Registrar, TaskState};
use crate::{CoreError, Result};

const NOT_FOUND: [HRESULT; 2] = [HRESULT(0x8007_0002_u32 as i32), HRESULT(0x8007_0003_u32 as i32)];

/// Task Scheduler through COM `ITaskService`. Each call runs on a thread of its own in the MTA, never
/// the caller's: an STA call pumps the UI thread's messages.
#[derive(Debug, Default)]
pub struct WindowsRegistrar;

fn on_mta<T: Send>(what: &str, f: impl FnOnce(&ITaskService) -> windows::core::Result<T> + Send) -> Result<T> {
    std::thread::scope(|scope| {
        scope
            .spawn(|| {
                // SAFETY: COM is initialized and uninitialized on this thread, around every COM object it makes.
                unsafe {
                    CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
                    let out = (|| {
                        let service: ITaskService = CoCreateInstance(&TaskScheduler, None, CLSCTX_INPROC_SERVER)?;
                        let none = VARIANT::default();
                        service.Connect(&none, &none, &none, &none)?;
                        f(&service)
                    })();
                    CoUninitialize();
                    out
                }
            })
            .join()
            .unwrap_or_else(|_| Err(windows::core::Error::from_hresult(HRESULT(0x8000_FFFF_u32 as i32))))
    })
    .map_err(|e| CoreError::Tasks(format!("{what}: {e}")))
}

/// `None` when `path` does not exist.
unsafe fn folder(service: &ITaskService, path: &str) -> windows::core::Result<Option<ITaskFolder>> {
    match unsafe { service.GetFolder(&BSTR::from(path)) } {
        Ok(f) => Ok(Some(f)),
        Err(e) if NOT_FOUND.contains(&e.code()) => Ok(None),
        Err(e) => Err(e),
    }
}

impl Registrar for WindowsRegistrar {
    fn user(&self) -> Result<String> {
        current_user_sid().map_err(|e| CoreError::Tasks(format!("the current user's SID: {e}")))
    }

    fn tasks(&self, path: &str) -> Result<Vec<(String, String)>> {
        on_mta(&format!("list {path}"), |service| unsafe {
            let Some(f) = folder(service, path)? else { return Ok(vec![]) };
            let all = f.GetTasks(TASK_ENUM_HIDDEN.0)?;
            let mut out = vec![];
            for i in 1..=all.Count()? {
                let task = all.get_Item(&VARIANT::from(i))?;
                out.push((task.Name()?.to_string(), task.Xml()?.to_string()));
            }
            Ok(out)
        })
    }

    fn register(&self, path: &str, name: &str, xml: &str) -> Result<()> {
        let full = format!(r"{path}\{name}");
        on_mta(&format!("register {full}"), |service| unsafe {
            let root = service.GetFolder(&BSTR::from(r"\"))?;
            let none = VARIANT::default();
            root.RegisterTask(&BSTR::from(&full), &BSTR::from(xml), TASK_CREATE_OR_UPDATE.0, &none, &none, TASK_LOGON_INTERACTIVE_TOKEN, &none)?;
            Ok(())
        })
    }

    fn delete(&self, path: &str, name: &str) -> Result<()> {
        on_mta(&format!(r"delete {path}\{name}"), |service| unsafe {
            match folder(service, path)? {
                Some(f) => f.DeleteTask(&BSTR::from(name), 0),
                None => Ok(()),
            }
        })
    }

    fn delete_empty_folder(&self, path: &str) -> Result<bool> {
        let Some((parent, name)) = path.rsplit_once('\\') else { return Ok(false) };
        let parent = if parent.is_empty() { r"\" } else { parent };
        on_mta(&format!("delete folder {path}"), |service| unsafe {
            let Some(f) = folder(service, path)? else { return Ok(true) };
            if f.GetTasks(TASK_ENUM_HIDDEN.0)?.Count()? > 0 || f.GetFolders(0)?.Count()? > 0 {
                return Ok(false);
            }
            let Some(p) = folder(service, parent)? else { return Ok(true) };
            p.DeleteFolder(&BSTR::from(name), 0)?;
            Ok(true)
        })
    }

    fn all_tasks(&self, skip: &(dyn Fn(&str) -> bool + Sync)) -> Result<Listing> {
        on_mta("list every task", |service| unsafe {
            let mut out = Listing::default();
            walk(&service.GetFolder(&BSTR::from(r"\"))?, skip, &mut out);
            Ok(out)
        })
    }
}

/// Reads only: `GetTasks`, `GetFolders` and each task's state; a folder or task that refuses is counted.
unsafe fn walk(folder: &ITaskFolder, skip: &(dyn Fn(&str) -> bool + Sync), out: &mut Listing) {
    let Ok(path) = (unsafe { folder.Path() }).map(|p| p.to_string()) else {
        out.unreadable += 1;
        return;
    };
    match unsafe { folder.GetTasks(TASK_ENUM_HIDDEN.0) } {
        Ok(all) => {
            for i in 1..=unsafe { all.Count() }.unwrap_or(0) {
                match unsafe { all.get_Item(&VARIANT::from(i)).and_then(|t| state(&path, &t)) } {
                    Ok(task) => out.tasks.push(task),
                    Err(_) => out.unreadable += 1,
                }
            }
        }
        Err(_) => out.unreadable += 1,
    }
    let Ok(children) = (unsafe { folder.GetFolders(0) }) else {
        out.unreadable += 1;
        return;
    };
    for i in 1..=unsafe { children.Count() }.unwrap_or(0) {
        let Ok(child) = (unsafe { children.get_Item(&VARIANT::from(i)) }) else {
            out.unreadable += 1;
            continue;
        };
        if unsafe { child.Path() }.is_ok_and(|p| skip(&p.to_string())) {
            continue;
        }
        unsafe { walk(&child, skip, out) };
    }
}

unsafe fn state(folder: &str, task: &IRegisteredTask) -> windows::core::Result<TaskState> {
    unsafe {
        Ok(TaskState {
            folder: folder.to_string(),
            name: task.Name()?.to_string(),
            xml: task.Xml()?.to_string(),
            enabled: task.Enabled()?.as_bool(),
            last_run_ms: ole_date_ms(task.LastRunTime().unwrap_or(0.0)),
            last_result: task.LastTaskResult().unwrap_or(0),
            next_run_ms: ole_date_ms(task.NextRunTime().unwrap_or(0.0)),
        })
    }
}

fn current_user_sid() -> windows::core::Result<String> {
    // SAFETY: plain Win32 calls on a token handle and buffers this function owns and frees.
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token)?;
        let mut len = 0u32;
        let _ = GetTokenInformation(token, TokenUser, None, 0, &mut len);
        let mut buf = vec![0u8; len as usize];
        let read = GetTokenInformation(token, TokenUser, Some(buf.as_mut_ptr().cast()), len, &mut len);
        let _ = CloseHandle(token);
        read?;
        let user = &*(buf.as_ptr() as *const TOKEN_USER);
        let mut text = PWSTR::null();
        ConvertSidToStringSidW(user.User.Sid, &mut text)?;
        let sid = text.to_string();
        let _ = LocalFree(Some(HLOCAL(text.0.cast())));
        sid.map_err(|_| windows::core::Error::from_hresult(HRESULT(0x8007_000D_u32 as i32)))
    }
}
