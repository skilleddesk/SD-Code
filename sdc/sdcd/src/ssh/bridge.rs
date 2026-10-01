//! A process that behaves like `ssh` and rides SDC's own connection (0.16.0).
//!
//! A turn's CLI on a host, an MCP server there, a process the agent leaves running and the Terminal are
//! all child processes whose stdin and stdout the daemon reads. They used to be `ssh -O proxy …` through
//! the ControlMaster. They are now `sdcd --ssh-bridge <port> <host> [--tty <cols>x<rows>] <command>`:
//! this same binary, which connects to the daemon over loopback, asks it for a channel on the host's
//! connection ([`super::native`]), and relays stdin, stdout, stderr and the exit code - so nothing that
//! starts, reads or kills these processes had to change.
//!
//! The loopback listener answers only a caller that knows the random token in
//! `<data>/ssh/bridge-<port>.token`, a file only this user can read.
//!
//! Frames, both ways: one byte of kind, four bytes of big-endian length, the bytes.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

use russh::ChannelMsg;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

use super::Ssh;

/// Client → daemon: stdin bytes.
const STDIN: u8 = 0;
/// Client → daemon: stdin is finished.
const STDIN_EOF: u8 = 1;
/// Daemon → client.
const STDOUT: u8 = 1;
const STDERR: u8 = 2;
/// Four bytes, big-endian `i32`: the exit code. The last frame.
const EXIT: u8 = 3;

/// The flag that makes `sdcd` this client instead of the daemon.
pub const FLAG: &str = "--ssh-bridge";

/// `(port, token)` of this daemon's listener, started on first use.
fn listener() -> Option<&'static (u16, String)> {
    static LISTENER: OnceLock<Option<(u16, String)>> = OnceLock::new();

    LISTENER
        .get_or_init(|| {
            let token = uuid::Uuid::new_v4().simple().to_string();
            let served = token.clone();

            let port = super::native::block(async move {
                let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.ok()?;
                let port = listener.local_addr().ok()?.port();

                tokio::spawn(async move {
                    while let Ok((socket, _)) = listener.accept().await {
                        let token = served.clone();

                        tokio::spawn(async move {
                            let _ = serve(socket, &token).await;
                        });
                    }
                });

                Some(port)
            })
            .flatten()?;

            let file = token_file(port)?;

            std::fs::write(&file, &token).ok()?;

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;

                let _ = std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600));
            }

            Some((port, token))
        })
        .as_ref()
}

fn token_file(port: u16) -> Option<PathBuf> {
    #[cfg(test)]
    let folder = std::env::temp_dir().join("sdc-bridge-test");
    #[cfg(not(test))]
    let folder = super::hostkey::known_hosts_path().ok()?.parent()?.to_path_buf();
    let _ = std::fs::create_dir_all(&folder);

    Some(folder.join(format!("bridge-{port}.token")))
}

/// The program and the arguments that run `command` on `ssh` through SDC's connection, the caller
/// appending the command line itself - the same shape as `ssh` + [`Ssh::base_args`]. `None` when SDC
/// does not hold a connection to this host.
pub fn launcher(ssh: &Ssh, tty: Option<(u64, u64)>) -> Option<(PathBuf, Vec<String>)> {
    if !super::native::known(ssh) {
        return None;
    }

    let (port, _) = listener()?;
    let program = std::env::current_exe().ok()?;
    let mut args = vec![FLAG.to_string(), port.to_string(), ssh.label()];

    if let Some((cols, rows)) = tty {
        args.push("--tty".to_string());
        args.push(format!("{cols}x{rows}"));
    }

    Some((program, args))
}

async fn read_frame<R: AsyncReadExt + Unpin>(reader: &mut R) -> Option<(u8, Vec<u8>)> {
    let kind = reader.read_u8().await.ok()?;
    let len = reader.read_u32().await.ok()? as usize;
    let mut data = vec![0; len];

    reader.read_exact(&mut data).await.ok()?;

    Some((kind, data))
}

async fn write_frame<W: AsyncWriteExt + Unpin>(writer: &mut W, kind: u8, data: &[u8]) -> std::io::Result<()> {
    writer.write_u8(kind).await?;
    writer.write_u32(data.len() as u32).await?;
    writer.write_all(data).await?;
    writer.flush().await
}

/// The daemon's side of one bridged process.
async fn serve(socket: tokio::net::TcpStream, token: &str) -> std::io::Result<()> {
    let (read, mut write) = socket.into_split();
    let mut read = BufReader::new(read);
    let mut header = String::new();

    read.read_line(&mut header).await?;

    let header: Value = serde_json::from_str(header.trim()).unwrap_or(Value::Null);

    if header["token"].as_str() != Some(token) {
        return Ok(());
    }

    let label = header["host"].as_str().unwrap_or_default().to_string();
    let command = header["command"].as_str().unwrap_or_default().to_string();
    let tty = header["tty"].as_array().map(|size| {
        (size.first().and_then(Value::as_u64).unwrap_or(100) as u32, size.get(1).and_then(Value::as_u64).unwrap_or(30) as u32)
    });

    let failed = |reason: String| format!("{}: Permission denied (keyboard-interactive). SDC's connection: {reason}\n", label.rsplit_once(':').map(|(host, _)| host).unwrap_or(&label));

    let (channel, permit) = match super::native::open_channel(&label, Duration::from_secs(120)).await {
        Ok(opened) => opened,
        Err(reason) => {
            write_frame(&mut write, STDERR, failed(reason).as_bytes()).await?;

            return write_frame(&mut write, EXIT, &255_i32.to_be_bytes()).await;
        }
    };

    if let Some((cols, rows)) = tty {
        let _ = channel.request_pty(false, "xterm-256color", cols, rows, 0, 0, &[]).await;
    }

    if let Err(error) = channel.exec(true, command.into_bytes()).await {
        write_frame(&mut write, STDERR, failed(error.to_string()).as_bytes()).await?;

        return write_frame(&mut write, EXIT, &255_i32.to_be_bytes()).await;
    }

    let (mut reader, writer) = channel.split();

    /* stdin: the process's own, relayed until it ends. The process dying closes the socket, which closes
       the channel - what `ssh` did when it was killed. */
    let stdin = tokio::spawn(async move {
        loop {
            match read_frame(&mut read).await {
                Some((STDIN, data)) => {
                    if writer.data_bytes(data).await.is_err() {
                        break;
                    }
                }
                Some((STDIN_EOF, _)) => {
                    let _ = writer.eof().await;
                }
                Some(_) => {}
                None => {
                    let _ = writer.close().await;

                    break;
                }
            }
        }
    });

    let mut code: Option<i32> = None;

    while let Some(message) = reader.wait().await {
        let sent = match message {
            ChannelMsg::Data { data } => write_frame(&mut write, STDOUT, &data).await,
            ChannelMsg::ExtendedData { data, .. } => write_frame(&mut write, STDERR, &data).await,
            ChannelMsg::ExitStatus { exit_status } => {
                code = Some(exit_status as i32);

                Ok(())
            }
            ChannelMsg::ExitSignal { .. } => {
                code = Some(255);

                Ok(())
            }
            ChannelMsg::Close => break,
            _ => Ok(()),
        };

        if sent.is_err() {
            break;
        }
    }

    stdin.abort();
    drop(permit);

    /* No exit status and no close: the connection itself went away, which `ssh` reports as 255. */
    let code = code.unwrap_or(255);

    if code == 255 && !super::native::is_live_label(&label) {
        let _ = write_frame(&mut write, STDERR, format!("Connection to {label} closed by remote host.\n").as_bytes()).await;
    }

    write_frame(&mut write, EXIT, &code.to_be_bytes()).await
}

/// `sdcd --ssh-bridge <port> <host> [--tty <cols>x<rows>] <command>`: the client. Returns the exit code.
pub fn client_main(args: &[String]) -> i32 {
    client_run(args, std::io::stdin(), &mut std::io::stdout(), &mut std::io::stderr())
}

/// [`client_main`] with its three streams given, so a test can run it in-process.
pub(crate) fn client_run<I: Read + Send + 'static>(args: &[String], mut input: I, stdout: &mut dyn Write, stderr: &mut dyn Write) -> i32 {
    let mut args = args.iter();
    let (Some(port), Some(host)) = (args.next(), args.next()) else {
        let _ = writeln!(stderr, "usage: sdcd {FLAG} <port> <host> [--tty <cols>x<rows>] <command>");

        return 255;
    };
    let mut rest: Vec<String> = args.cloned().collect();
    let mut tty = Value::Null;

    if rest.first().map(String::as_str) == Some("--tty") && rest.len() >= 2 {
        let size = rest[1].clone();
        let (cols, rows) = size.split_once('x').unwrap_or(("100", "30"));

        tty = json!([cols.parse::<u64>().unwrap_or(100), rows.parse::<u64>().unwrap_or(30)]);
        rest.drain(..2);
    }

    let command = rest.join(" ");
    let Ok(port) = port.parse::<u16>() else {
        return 255;
    };
    let token = token_file(port).and_then(|file| std::fs::read_to_string(file).ok()).unwrap_or_default();
    let Ok(socket) = std::net::TcpStream::connect(("127.0.0.1", port)) else {
        let _ = writeln!(stderr, "{host}: Connection closed: SDC is not running");

        return 255;
    };
    let _ = socket.set_nodelay(true);

    let Ok(mut sender) = socket.try_clone() else {
        return 255;
    };
    let header = json!({ "token": token.trim(), "host": host, "command": command, "tty": tty });

    if writeln!(sender, "{header}").is_err() {
        return 255;
    }

    std::thread::spawn(move || {
        let mut buffer = [0_u8; 16 * 1024];

        loop {
            match input.read(&mut buffer) {
                Ok(0) | Err(_) => {
                    let _ = sender.write_all(&[STDIN_EOF, 0, 0, 0, 0]);

                    break;
                }
                Ok(read) => {
                    let mut frame = Vec::with_capacity(read + 5);

                    frame.push(STDIN);
                    frame.extend_from_slice(&(read as u32).to_be_bytes());
                    frame.extend_from_slice(&buffer[..read]);

                    if sender.write_all(&frame).is_err() {
                        break;
                    }
                }
            }
        }
    });

    let mut reader = std::io::BufReader::new(socket);

    loop {
        let mut head = [0_u8; 5];

        if reader.read_exact(&mut head).is_err() {
            return 255;
        }

        let len = u32::from_be_bytes([head[1], head[2], head[3], head[4]]) as usize;
        let mut data = vec![0; len];

        if reader.read_exact(&mut data).is_err() {
            return 255;
        }

        match head[0] {
            STDOUT => {
                let _ = stdout.write_all(&data);
                let _ = stdout.flush();
            }
            STDERR => {
                let _ = stderr.write_all(&data);
                let _ = stderr.flush();
            }
            EXIT if data.len() == 4 => return i32::from_be_bytes([data[0], data[1], data[2], data[3]]),
            _ => {}
        }
    }
}
