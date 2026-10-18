//! Single-instance IPC over a per-user local socket: a Unix domain socket on
//! Linux and a named pipe on Windows. Each connection carries one JSON line.

use interprocess::local_socket::{ListenerOptions, Name, Stream, prelude::*};
use serde::{Deserialize, Serialize};
use std::io::{self, BufRead, BufReader, Read, Write};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

/// Upper bound for one command line; real commands are tiny.
const MAX_MESSAGE_BYTES: u64 = 64 * 1024;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Command {
    Toggle,
    Show,
    Hide,
    Settings,
    Query(String),
    Quit,
}

/// Try to deliver `cmd` to an already-running instance. Ok(true) if delivered.
pub fn send_to_running(cmd: &Command) -> io::Result<bool> {
    send_to(&socket_id(), cmd)
}

/// Become the primary instance: bind the socket (cleaning up a stale Unix socket file), spawn a
/// background thread that accepts connections, decodes one JSON line per connection, and forwards it.
/// Returns Err if another live instance already owns it.
pub fn serve() -> io::Result<UnboundedReceiver<Command>> {
    serve_on(&socket_id())
}

/// Per-user socket identifier: a file path on Unix, a pipe name on Windows.
fn socket_id() -> String {
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "user".into());
    let user = user
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        .collect::<String>();
    #[cfg(unix)]
    {
        // XDG_RUNTIME_DIR is private to the user; /tmp is the fallback.
        match std::env::var_os("XDG_RUNTIME_DIR").filter(|d| !d.is_empty()) {
            Some(dir) => std::path::Path::new(&dir)
                .join("spotlight.sock")
                .to_string_lossy()
                .into_owned(),
            None => format!("/tmp/spotlight-{user}.sock"),
        }
    }
    #[cfg(not(unix))]
    {
        // Named pipes are machine-global, so scope them by user name.
        format!("spotlight-{user}")
    }
}

fn socket_name(id: &str) -> io::Result<Name<'static>> {
    #[cfg(unix)]
    {
        use interprocess::local_socket::GenericFilePath;
        id.to_owned().to_fs_name::<GenericFilePath>()
    }
    #[cfg(not(unix))]
    {
        use interprocess::local_socket::GenericNamespaced;
        id.to_owned().to_ns_name::<GenericNamespaced>()
    }
}

fn send_to(id: &str, cmd: &Command) -> io::Result<bool> {
    let Ok(mut stream) = Stream::connect(socket_name(id)?) else {
        return Ok(false);
    };
    let mut line = serde_json::to_vec(cmd).map_err(io::Error::other)?;
    line.push(b'\n');
    stream.write_all(&line)?;
    stream.flush()?;
    Ok(true)
}

fn serve_on(id: &str) -> io::Result<UnboundedReceiver<Command>> {
    if Stream::connect(socket_name(id)?).is_ok() {
        return Err(io::Error::new(
            io::ErrorKind::AddrInUse,
            "another Spotlight instance is already running",
        ));
    }
    // Nobody answered, so any socket file left behind is from a crashed instance.
    #[cfg(unix)]
    let _ = std::fs::remove_file(id);
    let options = ListenerOptions::new().name(socket_name(id)?);
    #[cfg(unix)]
    let options = {
        use interprocess::os::unix::local_socket::ListenerOptionsExt;
        options.mode(0o600)
    };
    let listener = options.create_sync()?;
    let (tx, rx) = unbounded_channel();
    std::thread::Builder::new()
        .name("spotlight-ipc".into())
        .spawn(move || {
            for stream in listener.incoming() {
                if tx.is_closed() {
                    break;
                }
                let Ok(stream) = stream else { continue };
                let tx = tx.clone();
                // One short-lived thread per client so a stalled peer cannot
                // block later commands.
                let _ = std::thread::Builder::new()
                    .name("spotlight-ipc-client".into())
                    .spawn(move || handle_client(stream, &tx));
            }
        })?;
    Ok(rx)
}

fn handle_client(stream: Stream, tx: &UnboundedSender<Command>) {
    let mut line = String::new();
    if BufReader::new(stream.take(MAX_MESSAGE_BYTES))
        .read_line(&mut line)
        .is_err()
    {
        return;
    }
    match serde_json::from_str::<Command>(line.trim()) {
        Ok(command) => {
            let _ = tx.send(command);
        }
        Err(error) => tracing::debug!("ignoring malformed IPC message: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn test_id() -> String {
        let unique = format!("spotlight-test-{}", std::process::id());
        #[cfg(unix)]
        {
            std::env::temp_dir()
                .join(format!("{unique}.sock"))
                .to_string_lossy()
                .into_owned()
        }
        #[cfg(not(unix))]
        {
            unique
        }
    }

    #[tokio::test]
    async fn commands_round_trip_to_the_primary_instance() {
        let id = test_id();
        assert!(!send_to(&id, &Command::Show).unwrap());
        let mut rx = serve_on(&id).unwrap();
        assert!(serve_on(&id).is_err(), "second instance must not take over");
        for command in [Command::Query("fire fox".into()), Command::Toggle] {
            assert!(send_to(&id, &command).unwrap());
            let received = tokio::time::timeout(Duration::from_secs(5), rx.recv())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(received, command);
        }
        #[cfg(unix)]
        let _ = std::fs::remove_file(&id);
    }
}
