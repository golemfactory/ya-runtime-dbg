use crate::ui::*;
use actix::{Arbiter, System};
use anyhow::{Context, Result};
use futures::channel::{mpsc, oneshot};
use futures::{FutureExt, SinkExt, StreamExt};
use std::ffi::OsString;
use std::fs::create_dir_all;
use std::path::PathBuf;
use std::process::Stdio;
use std::str::FromStr;
use structopt::clap::arg_enum;
use structopt::{clap, StructOpt};
use tokio::process::Command;
use tokio_util::codec::{BytesCodec, FramedRead};
use ya_runtime_api::server::{spawn, ProcessStatus, RunProcess, RuntimeEvent, RuntimeService};

#[macro_use]
mod ui;

const ORGANIZATION: &str = "GolemFactory";
const QUALIFIER: &str = "";

/// Deploys and starts a runtime with an interactive prompt{n}
/// ya-runtime-dbg --runtime /usr/lib/yagna/plugins/ya-runtime-vm/ya-runtime-vm \{n}
/// --task-package /tmp/image.gvmi \{n}
/// --workdir /tmp/runtime \{n}
/// -- --cpu-cores 4
#[derive(StructOpt)]
#[structopt(global_setting = clap::AppSettings::ColoredHelp)]
#[structopt(global_setting = clap::AppSettings::DeriveDisplayOrder)]
#[structopt(rename_all = "kebab-case")]
struct Args {
    /// Mode to execute commands in
    #[structopt(
        long,
        possible_values = &ExecModeArg::variants(),
        case_insensitive = true,
        default_value = "shell"
    )]
    exec_mode: ExecModeArg,
    /// Execution shell (for "--exec-mode shell" or default mode)
    #[structopt(long, default_value = "bash")]
    exec_shell: String,
    /// Runtime binary
    #[structopt(short, long)]
    runtime: PathBuf,
    /// Working directory
    #[structopt(short, long)]
    workdir: PathBuf,
    /// Task package to deploy
    #[structopt(short, long)]
    task_package: PathBuf,
    /// Service protocol version
    #[structopt(short, long, default_value = "0.1.0")]
    protocol: String,
    /// Skip deployment phase
    #[structopt(
        long = "no-deploy",
        parse(from_flag = std::ops::Not::not),
    )]
    deploy: bool,
    /// Additional runtime arguments
    varargs: Vec<String>,
}

impl Args {
    fn to_runtime_args(&self) -> Vec<OsString> {
        let mut args = vec![
            OsString::from("--workdir"),
            self.workdir.clone().into_os_string(),
            OsString::from("--task-package"),
            self.task_package.clone().into_os_string(),
        ];
        args.extend(self.varargs.iter().map(OsString::from));
        args
    }
}

arg_enum! {
    #[derive(Clone, Copy, Debug)]
    enum ExecModeArg {
        Shell,
        Exec
    }
}

enum ExecMode {
    Shell(String),
    Exec,
}

impl ExecMode {
    fn new(mode_arg: ExecModeArg, shell: String) -> Self {
        match mode_arg {
            ExecModeArg::Shell => ExecMode::Shell(shell),
            ExecModeArg::Exec => ExecMode::Exec,
        }
    }
}

struct EventHandler<T: Terminal> {
    tx: mpsc::Sender<()>,
    arbiter: actix::Arbiter,
    ui: UI<T>,
}

impl<T: Terminal> EventHandler<T> {
    pub fn new(tx: mpsc::Sender<()>, ui: UI<T>) -> Self {
        let arbiter = Arbiter::current().clone();
        EventHandler { tx, ui, arbiter }
    }
}

impl<T: Terminal + 'static> RuntimeEvent for EventHandler<T> {
    fn on_process_status(&self, status: ProcessStatus) {
        if !status.stdout.is_empty() {
            write_output(&self.ui, status.stdout);
        }
        if !status.stderr.is_empty() {
            write_output(&self.ui, status.stderr);
        }
        if !status.running {
            match status.return_code {
                0 => (),
                c => ui_err!(self.ui, "command failed with code {}", c),
            }

            let mut tx = self.tx.clone();
            self.arbiter.send(
                async move {
                    let _ = tx.send(()).await;
                }
                .boxed(),
            );
        }
    }
}

fn forward_output<R, T>(read: R, mut writer: UI<T>)
where
    R: tokio::io::AsyncRead + 'static,
    T: Terminal + 'static,
{
    let stream = FramedRead::new(read, BytesCodec::new())
        .filter_map(|result| async { result.ok() })
        .ready_chunks(16)
        .map(|v| v.into_iter().map(|b| b.to_vec()).flatten().collect());
    Arbiter::spawn(async move {
        stream
            .for_each(move |v| futures::future::ready(write_output(&mut writer, v)))
            .await;
    });
}

fn write_output<T>(writer: &UI<T>, out: Vec<u8>)
where
    T: Terminal + 'static,
{
    let cow = String::from_utf8_lossy(out.as_slice());
    let out = cow.trim();
    if !out.is_empty() {
        let nl = if out.ends_with("\n") { "" } else { "\n" };
        write!(writer, "{}{}", out, nl).unwrap();
    }
}

async fn deploy<T>(args: &Args, ui: UI<T>) -> Result<()>
where
    T: Terminal + 'static,
{
    let mut rt_args = args.to_runtime_args();
    rt_args.push(OsString::from("deploy"));

    ui_info!(ui, "Deploying");

    let _ = create_dir_all(&args.workdir);
    let mut child = runtime_command(&args)?
        .kill_on_drop(true)
        .args(rt_args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    forward_output(child.stdout.take().unwrap(), ui.clone());
    forward_output(child.stderr.take().unwrap(), ui.clone());

    if !child.await?.success() {
        return Err(anyhow::anyhow!("deployment failed"));
    }

    writeln!(ui, "").unwrap();
    Ok(())
}

async fn start<T>(
    args: Args,
    mut input_rx: mpsc::Receiver<String>,
    start_tx: oneshot::Sender<()>,
    mut ui: UI<T>,
) -> Result<()>
where
    T: Terminal + 'static,
{
    let exec_mode = ExecMode::new(args.exec_mode, args.exec_shell.clone());
    let mut rt_args = args.to_runtime_args();
    rt_args.push(OsString::from("start"));

    ui_info!(ui, "Starting");

    let mut command = runtime_command(&args)?;
    command.args(rt_args);

    let (tx, mut rx) = mpsc::channel(1);
    let service = spawn(command, EventHandler::new(tx, ui.clone()))
        .await
        .context("unable to spawn runtime")?;

    // FIXME: handle hello result with newer version of runtime api
    let _ = service.hello(args.protocol.as_str()).await;

    ui_info!(ui, "Entering prompt, press C-d to exit");

    let _ = start_tx.send(());
    while let Some(input) = input_rx.next().await {
        if let Err(e) = run(service.clone(), input, &exec_mode).await {
            let message = e.root_cause().to_string();
            ui_err!(ui, "{}", message);
            // runtime apis do not allow us to recover from this error,
            // also do not provide machine-readable error codes
            if is_broken_pipe(&message) {
                ui_err!(ui, "Unrecoverable error, please restart");
                break;
            }
        } else {
            let _ = rx.next().await;
        }
    }

    ui.close();
    if let Err(e) = service.shutdown().await {
        let message = format!("{:?}", e);
        if !is_broken_pipe(&message) {
            ui_err!(ui, "Shutdown error: {}", message);
        }
    }

    Ok(())
}

async fn run(service: impl RuntimeService, input: String, mode: &ExecMode) -> Result<()> {
    let mut args = match mode {
        ExecMode::Shell(sh) => vec![format!("/bin/{}", sh), "-c".to_string(), input],
        ExecMode::Exec => {
            let args = shell_words::split(input.as_str())?;
            match args.len() {
                0 => return Ok(()),
                _ => args,
            }
        }
    };

    let bin_path = PathBuf::from_str(args.remove(0).as_str())?;
    let bin_name = bin_path
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("invalid command: {}", bin_path.display()))?
        .to_string_lossy()
        .to_string();

    let mut run_process = RunProcess::default();
    run_process.bin = bin_path.display().to_string();
    run_process.args = std::iter::once(bin_name)
        .chain(args.iter().map(|s| s.clone()))
        .collect();

    service
        .run_process(run_process)
        .await
        .map_err(|e| anyhow::anyhow!(e.message))?;

    Ok(())
}

fn runtime_command(args: &Args) -> Result<Command> {
    let rt_dir = args
        .runtime
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Invalid runtime parent directory"))?;
    let mut command = Command::new(&args.runtime);
    command.current_dir(rt_dir);
    Ok(command)
}

fn project_dir() -> Result<PathBuf> {
    let app_name = structopt::clap::crate_name!();
    let proj_dir = directories::ProjectDirs::from(QUALIFIER, ORGANIZATION, app_name)
        .map(|dirs| dirs.data_dir().into())
        .unwrap_or_else(|| PathBuf::from(ORGANIZATION).join(app_name));
    if !proj_dir.exists() {
        std::fs::create_dir_all(&proj_dir)
            .context(format!("Cannot create dir: {}", proj_dir.display()))?;
    }
    Ok(proj_dir)
}

fn is_broken_pipe(message: &str) -> bool {
    message.to_lowercase().find("error 32").is_some()
}

#[actix_rt::main]
async fn main() -> Result<()> {
    let mut args = Args::from_args();
    args.runtime = args.runtime.canonicalize().context("runtime not found")?;

    let proj_dir = project_dir()?;
    let history_path = proj_dir.join(".ya_dbg_history");
    let mut ui = default_ui(history_path)?;

    let rt_args = args
        .to_runtime_args()
        .into_iter()
        .map(|s: OsString| s.as_os_str().to_string_lossy().to_string())
        .collect::<Vec<_>>();

    ui_info!(
        ui,
        "Arguments: {} {}",
        args.runtime.display(),
        rt_args.join(" ")
    );

    if args.deploy {
        deploy(&args, ui.clone())
            .await
            .context("deployment failed")?;
    }

    let (start_tx, start_rx) = oneshot::channel();
    let (input_tx, input_rx) = mpsc::channel(1);

    std::thread::spawn({
        let ui = ui.clone();
        move || {
            System::new("runtime").block_on(async move {
                if let Err(e) = start(args, input_rx, start_tx, ui.clone()).await {
                    ui_err!(ui, "Runtime error: {}", e);
                }
            })
        }
    });

    start_rx.await?;
    ui.enter_prompt(input_tx).await;

    ui_info!(ui, "Shutting down");
    ui.close();
    Ok(())
}
