use std::env;
use std::process;
use std::thread;
use std::time::{Duration, Instant};

use device::connect;

fn parse_arguments() -> Result<(Option<String>, u64), String> {
    let mut arguments = env::args().skip(1);
    let mut port = None;
    let mut seconds = 1800_u64;
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--port" => {
                port = Some(
                    arguments
                        .next()
                        .ok_or_else(|| "--port requires a value".to_owned())?,
                );
            }
            "--seconds" => {
                seconds = arguments
                    .next()
                    .ok_or_else(|| "--seconds requires a value".to_owned())?
                    .parse()
                    .map_err(|_| "--seconds must be a positive integer".to_owned())?;
                if seconds == 0 {
                    return Err("--seconds must be positive".into());
                }
            }
            _ => return Err(format!("unknown option: {argument}")),
        }
    }
    Ok((port, seconds))
}

fn run() -> Result<(), String> {
    let (port, seconds) = parse_arguments()?;
    let mut connected = connect(port.as_deref()).map_err(|error| error.to_string())?;
    let mut heap_floor = connected.initial_status.free_heap;
    let mut previous_uptime = connected.initial_status.uptime_ms;
    let started = Instant::now();
    let deadline = started + Duration::from_secs(seconds);
    let mut heartbeats = 0_u64;
    let mut next_status = Duration::from_mins(1);
    while Instant::now() < deadline {
        let ack = connected
            .client
            .heartbeat()
            .map_err(|error| error.to_string())?;
        if ack.uptime_ms < previous_uptime {
            return Err("device uptime moved backwards (reset detected)".into());
        }
        previous_uptime = ack.uptime_ms;
        heartbeats += 1;
        if started.elapsed() >= next_status {
            let status = connected
                .client
                .status()
                .map_err(|error| error.to_string())?;
            heap_floor = heap_floor.min(status.free_heap);
            println!(
                "elapsed={}s uptime_ms={} free_heap={} valid={} malformed={} crc={} overflow={} dropped={}",
                started.elapsed().as_secs(),
                status.uptime_ms,
                status.free_heap,
                status.valid_frames,
                status.malformed_frames,
                status.crc_errors,
                status.overflow_frames,
                status.dropped_responses
            );
            next_status += Duration::from_mins(1);
        }
        thread::sleep(Duration::from_secs(1));
    }
    let final_status = connected
        .client
        .status()
        .map_err(|error| error.to_string())?;
    heap_floor = heap_floor.min(final_status.free_heap);
    println!(
        "PASS port={} elapsed={}s heartbeats={} uptime_ms={} heap_floor={} valid={} malformed={} crc={} overflow={} dropped={} rx_drops={}",
        connected.port_name,
        started.elapsed().as_secs(),
        heartbeats,
        final_status.uptime_ms,
        heap_floor,
        final_status.valid_frames,
        final_status.malformed_frames,
        final_status.crc_errors,
        final_status.overflow_frames,
        final_status.dropped_responses,
        final_status.rx_dropped_bytes
    );
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("heartbeat soak failed: {error}");
        process::exit(1);
    }
}
