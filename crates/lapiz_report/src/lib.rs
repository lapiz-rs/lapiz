use std::{
    backtrace::Backtrace,
    cmp::Reverse,
    fmt::{self, Display, Write},
    fs,
    panic::{self, Location},
    sync::LazyLock,
};

use anyhow::anyhow;
use chrono::{Local, Utc};
#[cfg(not(target_os = "android"))]
use gfxinfo::active_gpu;
use lapiz_dirs::panic_reports_dir;
#[cfg(not(target_os = "android"))]
use lapiz_runtime::renderer::global_render_context;
use lapiz_utils::log_err::LogErr as _;
use sysinfo::{System, get_current_pid};
use wgpu::{AllocatorReport, Device};

static EMOTICONS_LIST: LazyLock<Vec<&'static str>> =
    LazyLock::new(|| include_str!("emoticons.txt").lines().collect());

pub fn setup_panic_hook() {
    panic::set_hook(Box::new(|info| {
        let Ok(mut report) = panic_report(
            info.payload_as_str(),
            info.location(),
            Backtrace::force_capture(),
        )
        .logged_err() else {
            return;
        };

        sysinfo_report(&mut report).log_err();
        #[cfg(not(target_os = "android"))]
        wgpu_report(&mut report, &global_render_context().device).log_err();

        log::error!("{}", report);
        fs::write(
            panic_reports_dir().join(format!(
                "panic-{}.txt",
                Utc::now().to_rfc3339().replace(':', "-")
            )),
            report,
        )
        .log_err();
    }));
}

pub fn panic_report(
    payload: Option<&str>,
    location: Option<&Location>,
    backtrace: Backtrace,
) -> anyhow::Result<String> {
    let mut buf = String::new();
    let w = &mut buf;

    writeln!(w, "Program panicked at {}", Utc::now().to_rfc3339())?;
    if let Some(location) = location {
        writeln!(
            w,
            "in {} at {}:{}",
            location.file(),
            location.line(),
            location.column()
        )?;
    } else {
        writeln!(w, "at unknown location")?;
    }
    writeln!(w, "{}", payload.unwrap_or("<no string payload available>"))?;

    writeln!(w)?;
    writeln!(
        w,
        "{}",
        EMOTICONS_LIST[rand::random_range(0..EMOTICONS_LIST.len())]
    )?;

    writeln!(w)?;
    writeln!(w, "Stacktrace:\n{}", backtrace)?;

    Ok(buf)
}

pub fn report(device: &Device) -> anyhow::Result<String> {
    let mut buf = String::new();

    let w = &mut buf;
    writeln!(w, "Generated at {}", Local::now())?;

    sysinfo_report(w)?;
    wgpu_report(w, device)?;
    Ok(buf)
}

fn sysinfo_report(w: &mut dyn Write) -> anyhow::Result<()> {
    let sys = System::new_all();
    writeln!(w, "System Info")?;

    writeln!(w, "System CPUs")?;
    for cpu in sys.cpus().iter() {
        writeln!(
            w,
            "  {} {}@{}Hz {}%",
            cpu.name(),
            cpu.brand(),
            cpu.frequency(),
            cpu.cpu_usage()
        )?;
    }

    writeln!(
        w,
        "System Memory: {} / {}",
        FmtBytes(sys.used_memory()),
        FmtBytes(sys.total_memory())
    )?;

    writeln!(
        w,
        "System Swap: {} / {}",
        FmtBytes(sys.used_swap()),
        FmtBytes(sys.total_swap())
    )?;

    #[cfg(not(target_os = "android"))]
    {
        let gpu = active_gpu().map_err(|e| anyhow!("{}", e))?;
        let gpu_info = gpu.info();
        writeln!(w, "System GPU: {} {}%", gpu.model(), gpu_info.load_pct())?;
        writeln!(
            w,
            "System VRAM: {} / {}",
            FmtBytes(gpu_info.used_vram()),
            FmtBytes(gpu_info.total_vram())
        )?;
    }

    let cur_pid = get_current_pid().map_err(|e| anyhow!("{}", e))?;
    let process = sys.process(cur_pid).unwrap();

    writeln!(w, "Process CPU: {}%", process.cpu_usage())?;
    writeln!(w, "Process Memory: {}", FmtBytes(process.memory()))?;

    writeln!(w)?;

    Ok(())
}

fn wgpu_report(w: &mut dyn Write, device: &Device) -> anyhow::Result<()> {
    let Some(AllocatorReport {
        mut allocations,
        blocks,
        total_allocated_bytes,
        total_reserved_bytes,
    }) = device.generate_allocator_report()
    else {
        writeln!(w, "No WGPU report available")?;
        return Ok(());
    };
    writeln!(w, "WGPU Report")?;

    allocations.sort_by_key(|alloc| Reverse(alloc.size));

    writeln!(
        w,
        "Summary: {} / {}",
        FmtBytes(total_allocated_bytes),
        FmtBytes(total_reserved_bytes)
    )?;
    writeln!(w, "Blocks: {}", blocks.len())?;
    writeln!(w, "Allocations: {}", allocations.len())?;
    for (i, alloc) in allocations.iter().enumerate() {
        writeln!(w, "  #{} {}: {}", i, alloc.name, FmtBytes(alloc.size))?;
    }

    writeln!(w)?;

    Ok(())
}

struct FmtBytes(u64);

impl Display for FmtBytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        const SUFFIX: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
        let mut idx = 0;
        let mut amount = self.0 as f64;
        loop {
            if amount < 1024.0 || idx == SUFFIX.len() - 1 {
                return write!(f, "{:.2} {} ({} bytes)", amount, SUFFIX[idx], self.0);
            }

            amount /= 1024.0;
            idx += 1;
        }
    }
}
