use std::ffi::{OsString, c_void};
use std::future::Future;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
use windows_service::service::{
    ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus, ServiceType,
};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
use windows_service::{define_windows_service, service_dispatcher};
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, LocalFree};
use windows_sys::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::{
    GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation, IsWellKnownSid, SECURITY_ATTRIBUTES,
    TOKEN_ELEVATION, TOKEN_QUERY, TOKEN_USER, TokenElevation, TokenUser, WinLocalSystemSid,
};
use windows_sys::Win32::System::Pipes::GetNamedPipeClientProcessId;
use windows_sys::Win32::System::Threading::{OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION};

use crate::daemon::Daemon;

pub const SERVICE_NAME: &str = "fluidityd";
const PIPE_SDDL: &str = "D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GRGW;;;AU)";

pub struct PipeSecurity {
    descriptor: *mut c_void,
    attributes: SECURITY_ATTRIBUTES,
}

unsafe impl Send for PipeSecurity {}

impl PipeSecurity {
    pub fn new() -> std::io::Result<Self> {
        let sddl: Vec<u16> = PIPE_SDDL.encode_utf16().chain(std::iter::once(0)).collect();
        let mut descriptor = std::ptr::null_mut();
        let ok = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(Self {
            descriptor,
            attributes: SECURITY_ATTRIBUTES {
                nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: descriptor,
                bInheritHandle: 0,
            },
        })
    }

    pub fn create(&mut self, name: &std::ffi::OsStr, first: bool) -> std::io::Result<NamedPipeServer> {
        let attributes = &mut self.attributes as *mut SECURITY_ATTRIBUTES as *mut c_void;
        unsafe {
            ServerOptions::new()
                .first_pipe_instance(first)
                .reject_remote_clients(true)
                .create_with_security_attributes_raw(name, attributes)
        }
    }
}

impl Drop for PipeSecurity {
    fn drop(&mut self) {
        unsafe {
            LocalFree(self.descriptor);
        }
    }
}

fn token_information<T>(token: HANDLE, class: i32) -> Option<Vec<u8>> {
    let mut size = 0u32;
    unsafe { GetTokenInformation(token, class, std::ptr::null_mut(), 0, &mut size) };
    if (size as usize) < std::mem::size_of::<T>() {
        return None;
    }
    let mut buffer = vec![0u8; size as usize];
    let ok = unsafe { GetTokenInformation(token, class, buffer.as_mut_ptr() as *mut c_void, size, &mut size) };
    (ok != 0).then_some(buffer)
}

pub fn client_uid(pipe: &NamedPipeServer) -> Option<u32> {
    use std::os::windows::io::AsRawHandle;
    let mut pid = 0u32;
    if unsafe { GetNamedPipeClientProcessId(pipe.as_raw_handle() as HANDLE, &mut pid) } == 0 {
        return None;
    }
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return None;
    }
    let mut token: HANDLE = std::ptr::null_mut();
    let opened = unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) };
    unsafe { CloseHandle(process) };
    if opened == 0 {
        return None;
    }
    let uid = (|| {
        let user = token_information::<TOKEN_USER>(token, TokenUser)?;
        let sid = unsafe { (*(user.as_ptr() as *const TOKEN_USER)).User.Sid };
        if unsafe { IsWellKnownSid(sid, WinLocalSystemSid) } != 0 {
            return Some(crate::server::ROOT);
        }
        if let Some(elevation) = token_information::<TOKEN_ELEVATION>(token, TokenElevation)
            && unsafe { (*(elevation.as_ptr() as *const TOKEN_ELEVATION)).TokenIsElevated } != 0
        {
            return Some(crate::server::ROOT);
        }
        let count = unsafe { *GetSidSubAuthorityCount(sid) };
        if count == 0 {
            return None;
        }
        let rid = unsafe { *GetSidSubAuthority(sid, count as u32 - 1) };
        Some(if rid == crate::server::ROOT { u32::MAX } else { rid })
    })();
    unsafe { CloseHandle(token) };
    uid
}

pub fn secure_dir(dir: &Path) {
    let _ = std::process::Command::new("icacls")
        .arg(dir)
        .args([
            "/inheritance:r",
            "/grant:r",
            "*S-1-5-18:(OI)(CI)F",
            "/grant:r",
            "*S-1-5-32-544:(OI)(CI)F",
            "/Q",
        ])
        .output();
}

pub async fn listen(daemon: Arc<Daemon>, pipe: &Path, stop: impl Future<Output = ()>) -> std::io::Result<()> {
    let name = pipe.as_os_str();
    let mut security = PipeSecurity::new()?;
    let mut server = security.create(name, true)?;
    tracing::info!("listening on {}", pipe.display());
    tokio::pin!(stop);
    loop {
        tokio::select! {
            connected = server.connect() => {
                connected?;
                let client = std::mem::replace(&mut server, security.create(name, false)?);
                match client_uid(&client) {
                    Some(uid) => {
                        tokio::spawn(crate::server::serve(daemon.clone(), client, uid));
                    }
                    None => tracing::warn!("rejected a client with an unknown identity"),
                }
            }
            _ = &mut stop => break,
        }
    }
    crate::shutdown(daemon).await;
    Ok(())
}

define_windows_service!(ffi_service_main, service_main);

pub fn run_service() -> Result<(), windows_service::Error> {
    service_dispatcher::start(SERVICE_NAME, ffi_service_main)
}

fn service_main(_arguments: Vec<OsString>) {
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
    let stop_tx = std::sync::Mutex::new(Some(stop_tx));
    let handler = move |control| match control {
        ServiceControl::Stop | ServiceControl::Shutdown => {
            if let Some(tx) = stop_tx.lock().ok().and_then(|mut t| t.take()) {
                let _ = tx.send(());
            }
            ServiceControlHandlerResult::NoError
        }
        ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
        _ => ServiceControlHandlerResult::NotImplemented,
    };
    let Ok(status) = service_control_handler::register(SERVICE_NAME, handler) else {
        return;
    };
    report(
        status,
        ServiceState::Running,
        ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
        0,
    );
    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build();
    if let Ok(runtime) = runtime {
        runtime.block_on(crate::run(None, async move {
            let _ = stop_rx.await;
            report(status, ServiceState::StopPending, ServiceControlAccept::empty(), 15);
        }));
    }
    report(status, ServiceState::Stopped, ServiceControlAccept::empty(), 0);
}

fn report(
    status: service_control_handler::ServiceStatusHandle,
    state: ServiceState,
    accept: ServiceControlAccept,
    wait: u64,
) {
    let _ = status.set_service_status(ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: state,
        controls_accepted: accept,
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::from_secs(wait),
        process_id: None,
    });
}
