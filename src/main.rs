use core::mem;
use std::process::exit;
use std::sync::{Arc, Mutex};
// use std::thread;
// use std::time::Duration;
use itertools::Itertools;

use clap::Parser;
use jack::*;
use nix::sys::signalfd::signal::{signal, SigHandler, Signal};
use serde_json;

/// Jack VU-Meter inspired by cadence-jackmeter
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(long = "json")]
    json: bool,
    port: Vec<String>,
}

fn main() {
    unsafe { signal(Signal::SIGHUP, SigHandler::SigIgn) }.unwrap();

    let args: Args = Args::parse();
    let client = create_client().expect("Failed to create Jack client");
    let ports = match connect_ports(&client, &args.port) {
        Ok(ports) => ports,
        Err(err) => {
            eprintln!("Failed to connect ports: {err:#?}");
            exit(1);
        }
    };

    let process_handler_context = ProcessHandlerContext::new(ports);
    let vu = process_handler_context.vu();

    // let frame_dur_ms = 1000 * client.buffer_size() / client.sample_rate() as u32;

    let _ac = match client.activate_async((), process_handler_context) {
        Ok(ac) => ac,
        Err(e) => {
            eprintln!("Failed to activate {:?}", e);
            return;
        }
    };

    if args.json {
        println!("{}", serde_json::to_string(&args.port).unwrap());
    }

    let n_chan = vu.lock().unwrap().len();

    loop {
        let mut ch = vec![0f32; n_chan];
        {
            let mut src = vu.lock().unwrap();
            mem::swap(&mut ch, &mut *src);
        }
        println!("{}", ch.iter().map(|x| format!("{:.3}", x)).join(" "));
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

/*
fn interp_i(a: i16, b: i16, pos: usize, max_pos: usize) -> i16 {
    (
        (
            a as i32 * (max_pos - pos) as i32
                +
                b as i32 * pos as i32
        ) / max_pos as i32
    ) as i16
}

fn interp_f(a: i16, b: i16, pos: f32) -> f32 {
    a as f32 * (1f32 - pos)
        +
        b as f32 * pos
}
*/

fn create_client() -> Result<Client, Error> {
    let options = ClientOptions::NO_START_SERVER /* | ClientOptions::USE_EXACT_NAME */;
    let (client, status) = Client::new("VU meter", options)?;
    if !(status & ClientStatus::NAME_NOT_UNIQUE).is_empty() {
        println!("We are not alone!");
    }
    Ok(client)
}

fn connect_ports(client: &Client, ports: &Vec<String>) -> Result<Vec<Port<AudioIn>>, Error> {
    let mut dst_ports = Vec::<Port<AudioIn>>::new();
    // let num_channels = dst_ports.len();
    ports
        .iter()
        .enumerate()
        .map(|(num, arg)| {
            let optional = arg.ends_with('?');
            let arg = if optional { &arg[..arg.len() - 1] } else { arg };
            let mut s = arg.splitn(1, ':');
            let port = s.next().expect("Missing port");
            (num + 1, port, optional)
        })
        .filter_map(|(dst_channel, src_port_name, optional)| {
            let src_port = match client.port_by_name(src_port_name) {
                Some(p) => p,
                None => {
                    eprintln!(
                        "No such port `{}` to connect to channel {}",
                        src_port_name, dst_channel
                    );
                    eprintln!("Available:");
                    for port in
                        client.ports(None, Some(AudioOut.jack_port_type()), PortFlags::IS_OUTPUT)
                    {
                        eprintln!("  - `{}`", port);
                    }
                    if optional {
                        return None;
                    } else {
                        panic!("Bad port name");
                    }
                }
            };
            if !src_port.flags().contains(PortFlags::IS_OUTPUT) {
                panic!("Port `{}` is not an output port!", src_port_name);
            }
            let dst_port = client
                .register_port(&format!("in_{}", dst_channel + 1), AudioIn::default())
                .unwrap_or_else(|_| panic!("Failed to register port {}", dst_channel));
            let src_port_type = src_port.port_type().unwrap();
            let dst_port_type = dst_port.port_type().unwrap();
            if src_port_type != dst_port_type {
                panic!(
                    "Port `{n}` has wrong type — expected {e} but got {a}",
                    n = src_port_name,
                    e = dst_port_type,
                    a = src_port_type
                );
            }
            Some((src_port_name, src_port, dst_channel, dst_port, optional))
        })
        .for_each(
            |(src_port_name, src_port, dst_channel, dst_port, optional)| {
                client
                    .connect_ports(&src_port, &dst_port)
                    .unwrap_or_else(|e| {
                        eprintln!(
                            "Failed to connect port `{}` to channel {}: {:#?}",
                            src_port_name, dst_channel, e
                        );
                        if !optional {
                            panic!("Bad connection");
                        }
                    });
                dst_ports.push(dst_port);
            },
        );
    Ok(dst_ports)
}

struct ProcessHandlerContext {
    vu: Arc<Mutex<Vec<f32>>>,
    ports: Vec<Port<AudioIn>>,
}

impl ProcessHandlerContext {
    fn new(ports: Vec<Port<AudioIn>>) -> ProcessHandlerContext {
        ProcessHandlerContext {
            vu: Arc::new(Mutex::new(vec![0f32; ports.len()])),
            ports: ports,
        }
    }

    fn vu(&self) -> Arc<Mutex<Vec<f32>>> {
        Arc::clone(&self.vu)
    }
}

impl ProcessHandler for ProcessHandlerContext {
    fn process(&mut self, _client: &Client, ps: &ProcessScope) -> Control {
        let mut vu = self.vu.lock().unwrap();
        self.ports.iter().enumerate().for_each(|(i, chan)| {
            let max_of_chan = chan
                .as_slice(ps)
                .iter()
                .map(|s| s.abs())
                .max_by(|a, b| a.partial_cmp(b).unwrap())
                .unwrap();
            vu[i] = vu[i].max(max_of_chan);
        });
        Control::Continue
    }
}
