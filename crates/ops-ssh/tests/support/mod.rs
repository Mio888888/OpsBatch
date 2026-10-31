//! Integration fixture: a private, user-level OpenSSH server on 127.0.0.1 (pubkey auth only, no passwords, no
//! keychain), a fresh data dir and one registered host. Tests are skipped when sshd/ssh-keygen are unavailable.

#![allow(dead_code)]

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use ops_ssh::hosts::{add_host, NewHost};
use ops_ssh::Ctx;

pub struct Sshd {
    pub dir: PathBuf,
    pub port: u16,
    pub private_key: String,
    child: Child,
}

impl Drop for Sshd {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

impl Sshd {
    pub fn start() -> Option<Self> {
        let sshd = ["/usr/sbin/sshd", "/usr/bin/sshd"]
            .into_iter()
            .find(|p| std::path::Path::new(p).exists())?;
        let sftp = [
            "/usr/libexec/sftp-server",
            "/usr/lib/openssh/sftp-server",
            "/usr/lib/ssh/sftp-server",
        ]
        .into_iter()
        .find(|p| std::path::Path::new(p).exists())?;
        let dir = std::env::temp_dir().join(format!("opsbatch-sshd-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).ok()?;
        for name in ["key", "hostkey"] {
            let ok = Command::new("ssh-keygen")
                .args(["-q", "-t", "ed25519", "-N", "", "-f"])
                .arg(dir.join(name))
                .status()
                .ok()?
                .success();
            if !ok {
                return None;
            }
        }
        std::fs::copy(dir.join("key.pub"), dir.join("authorized_keys")).ok()?;
        let port = free_port();
        let config = format!(
            "Port {port}\nListenAddress 127.0.0.1\nHostKey {d}/hostkey\nAuthorizedKeysFile {d}/authorized_keys\n\
             PasswordAuthentication no\nPubkeyAuthentication yes\nKbdInteractiveAuthentication no\nUsePAM no\n\
             StrictModes no\nPidFile {d}/sshd.pid\nSubsystem sftp {sftp}\nAllowTcpForwarding yes\nPermitTTY yes\n\
             LogLevel ERROR\n",
            d = dir.display()
        );
        std::fs::write(dir.join("sshd_config"), config).ok()?;
        let child = Command::new(sshd)
            .args(["-D", "-e", "-f"])
            .arg(dir.join("sshd_config"))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let private_key = std::fs::read_to_string(dir.join("key")).ok()?;
        let this = Self {
            dir,
            port,
            private_key,
            child,
        };
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
                return Some(this);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        None
    }
}

pub struct Fixture {
    pub ctx: Arc<Ctx>,
    pub sshd: Sshd,
    pub host_id: String,
    pub work: PathBuf,
}

/// None => environment lacks OpenSSH; the calling test returns early (and says so).
pub async fn fixture(label: &str) -> Option<Fixture> {
    let Some(sshd) = Sshd::start() else {
        eprintln!("SKIP {label}: no usable sshd/ssh-keygen on this machine");
        return None;
    };
    let core = ops_core::Core::open_temp(label).expect("core");
    let ctx = Ctx::new(core);
    let user = std::env::var("USER").unwrap_or_else(|_| "root".into());
    let host = add_host(
        &ctx.db,
        NewHost {
            name: "local-sshd".into(),
            ip: "127.0.0.1".into(),
            port: Some(sshd.port as i32),
            auth_type: Some("key".into()),
            username: Some(user),
            password: None,
            private_key: Some(sshd.private_key.clone()),
            os: Some("linux".into()),
            tags: None,
            group_id: None,
            remark: None,
            jump_chain: None,
            rdp_settings: None,
            proxy_settings: None,
        },
    )
    .await
    .expect("add_host");
    let work = std::env::temp_dir().join(format!("opsbatch-work-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&work).unwrap();
    Some(Fixture {
        ctx,
        host_id: host.id,
        sshd,
        work,
    })
}
