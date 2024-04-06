use core::mem;
use std::process::exit;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use clap::Parser;
use jack::*;
use nix::sys::signalfd::signal::{SigHandler, signal, Signal};

/// Jack VU-Meter inspired by cadence-jackmeter
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Sets the number of input channels
    #[arg(short, long, default_value_t=2)]
    channels: usize,

    /// Automatically connect ports to vu-meter on startup. Format is `channel:port` where `channel` is the VU meter channel number starting from 1 and `port` is the output port to connect to. Can be given any number of times.
    #[arg(short='C', long)]
    connect: Vec<String>,
}

fn main() {
    unsafe { signal(Signal::SIGHUP, SigHandler::SigIgn) }.unwrap();

    let args: Args = Args::parse();
    let num_channels = args.channels;

    let client = create_client().expect("Failed to create Jack client");
    let client_name = client.name().to_string();
    let ports = setup_ports(&client, num_channels);

    let process_handler_context = ProcessHandlerContext::new(
        ports,
    );

    let vu = process_handler_context.vu();

    let notification_handler_context = NotificationHandlerContext { };

    let frame_dur_ms = 1000 * client.buffer_size() / client.sample_rate() as u32;

    let ac = match client.activate_async(notification_handler_context, process_handler_context) {
        Ok(ac) => ac,
        Err(e) => {
            eprintln!("Failed to activate {:?}", e);
            return;
        }
    };

    if let Err(err) = connect_ports(client_name, &ac, args.connect, num_channels) {
        eprintln!("Failed to connect ports: {err:#?}");
        exit(1);
    }

    let (conn, screen_num) = match xcb::Connection::connect(None) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to connect to X server: {:?}", e);
            exit(1);
        }
    };
    let conn = Arc::new(conn);
    let screen = conn.get_setup().roots().nth(screen_num as usize).unwrap();

    let colormap = screen.default_colormap();

    let gc_bg         = GcState::new(&conn, &screen, colormap, 0x000000);
    let gc_meter_low  = GcState::new(&conn, &screen, colormap, 0x5DE73D);
    let gc_meter_med  = GcState::new(&conn, &screen, colormap, 0xFFFF00);
    let gc_meter_high = GcState::new(&conn, &screen, colormap, 0xFF0000);
    let gc_grid_low   = GcState::new(&conn, &screen, colormap, 0x062806);
    let gc_grid_med1  = GcState::new(&conn, &screen, colormap, 0x282806);
    let gc_grid_med2  = GcState::new(&conn, &screen, colormap, 0x472806);
    let gc_grid_high  = GcState::new(&conn, &screen, colormap, 0x280F06);

    let mut win_w: u16 = 108;
    let mut win_h: u16 = 204;

    let title = "VU meter";

    let win = conn.generate_id();
    xcb::create_window(&conn,
                       xcb::COPY_FROM_PARENT as u8,
                       win,
                       screen.root(),
                       0, 0,
                       win_w, win_h,
                       10,
                       xcb::WINDOW_CLASS_INPUT_OUTPUT as u16,
                       screen.root_visual(), &[
            //(xcb::CW_BACK_PIXEL, screen.black_pixel()),
            (xcb::CW_EVENT_MASK,
             xcb::EVENT_MASK_EXPOSURE |
                 xcb::EVENT_MASK_STRUCTURE_NOTIFY
            ),
        ]
    );
    xcb::map_window(&conn, win);
    xcb::change_property(&conn, xcb::PROP_MODE_REPLACE as u8, win,
                         xcb::ATOM_WM_NAME, xcb::ATOM_STRING, 8, title.as_bytes());

    {
        // thread sending expose events at even intervals
        let conn = conn.clone();
        thread::spawn(move || {
            let refresh = Duration::from_millis(frame_dur_ms.max(50) as u64);
            loop {
                let event = xcb::ExposeEvent::new(win, 0, 0, 0, 0, 0);
                xcb::send_event(&conn, true, win, xcb::EVENT_MASK_EXPOSURE, &event);
                //xcb::clear_area(&conn, true, win, 0, 0, 10000, 10000);
                conn.flush();
                thread::sleep(refresh);
            }
        });
    }

    let gc_bg = gc_bg.finalize();
    let gc_meter_low = gc_meter_low.finalize();
    let gc_meter_med = gc_meter_med.finalize();
    let gc_meter_high = gc_meter_high.finalize();
    let gc_grid_low = gc_grid_low.finalize();
    let gc_grid_med1 = gc_grid_med1.finalize();
    let gc_grid_med2 = gc_grid_med2.finalize();
    let gc_grid_high = gc_grid_high.finalize();

    conn.flush();

    let mut prev_ch = vec![];
    let mut prev_locations = vec![];
    loop {
        let event = conn.wait_for_event();
        match event {
            None => { break; }
            Some(event) => {
                let r = event.response_type() & !0x80;
                match r {
                    xcb::EXPOSE => {
                        let event : &xcb::ExposeEvent = unsafe {
                            xcb::cast_event(&event)
                        };
                        let is_fake_expose = event.width() == 0 && event.height() == 0;

                        let mut ch = vec![0f32; num_channels];
                        let ch = {
                            let mut src = vu.lock().unwrap();
                            mem::swap(&mut ch, &mut *src);
                            if is_fake_expose && ch == prev_ch {
                                continue;
                            }
                            ch
                        };
                        prev_ch = ch.clone();
                        /*
                        let evt_x0 = event.x();
                        let evt_y0 = event.y();
                        let evt_x1 = evt_x0 + event.width() - 1;
                        let evt_y1 = evt_y0 + event.height() - 1;
                        println!("Expose {},{} - {},{}", evt_x0, evt_y0, evt_x1, evt_y1);
                         */
                        let x: (i16, i16) = (0, win_w as i16 - 1);
                        let y: (i16, i16) = (0, win_h as i16 - 1);

                        // bar chart
                        let locations = ch.iter()
                            .enumerate()
                            .map(|(i, level)| {
                                let x0 = interp_i(x.0, x.1, i, num_channels);
                                let x1 = interp_i(x.0, x.1, i + 1, num_channels);
                                let yp = interp_f(y.1 + 1, y.0, *level) as i16;
                                let y = if *level < 0.7 {
                                    [y.0, yp, yp, yp, y.1 + 1]
                                } else {
                                    let ym1 = interp_f(y.1 + 1, y.0, 0.7) as i16;
                                    if *level < 0.9 {
                                        [y.0, yp, yp, ym1, y.1 + 1]
                                    } else {
                                        let ym2 = interp_f(y.1 + 1, y.0, 0.9) as i16;
                                        [y.0, yp, ym2, ym1, y.1 + 1]
                                    }
                                };
                                (x0, x1, y)
                            })
                            .collect::<Vec<(i16, i16, [i16; 5])>>();

                        if is_fake_expose && locations == prev_locations {
                            continue;
                        }
                        prev_locations = locations.clone();

                        for (i, gc) in [gc_bg, gc_meter_high, gc_meter_med, gc_meter_low].iter().enumerate() {
                            let r: Vec<xcb::Rectangle> = locations.iter().flat_map(
                                |(x0, x1, y)|
                                    rect(*x0, *x1, y[i], y[i+1]-1)
                            ).collect();
                            if !r.is_empty() {
                                xcb::poly_fill_rectangle(&conn, win, *gc, &r);
                            }
                        }

                        // grid
                        let y1 = interp_f(y.1, y.0, 0.25) as i16;
                        let y2 = interp_f(y.1, y.0, 0.5) as i16;
                        let y3 = interp_f(y.1, y.0, 0.7) as i16;
                        let y4 = interp_f(y.1, y.0, 0.83) as i16;
                        let y5 = interp_f(y.1, y.0, 0.9) as i16;
                        let y6 = interp_f(y.1, y.0, 0.96) as i16;
                        xcb::poly_segment(&conn, win, gc_grid_low, &[
                            xcb::Segment::new(x.0, y1, x.1, y1),
                            xcb::Segment::new(x.0, y2, x.1, y2),
                        ]);
                        xcb::poly_segment(&conn, win, gc_grid_med1, &[
                            xcb::Segment::new(x.0, y3, x.1, y3),
                            xcb::Segment::new(x.0, y4, x.1, y4),
                        ]);
                        xcb::poly_segment(&conn, win, gc_grid_med2, &[
                            xcb::Segment::new(x.0, y5, x.1, y5),
                        ]);
                        xcb::poly_segment(&conn, win, gc_grid_high, &[
                            xcb::Segment::new(x.0, y6, x.1, y6),
                        ]);

                        conn.flush();
                    },
                    xcb::CONFIGURE_NOTIFY => {
                        let event: &xcb::ConfigureNotifyEvent = unsafe {
                            xcb::cast_event(&event)
                        };
                        win_w = event.width();
                        win_h = event.height();
                        //println!("Resize: {} x {}", win_w, win_h);
                    },
                    _ => {}
                }
            }
        }
    }
}

struct GcState<'a,'b> {
    cookie: xcb::AllocColorCookie<'a>,
    screen: &'b xcb::Screen<'b>,
}

impl<'a,'b> GcState<'a,'b> {
    fn new(conn: &'a xcb::Connection, screen: &'b xcb::Screen, colormap: xcb::Colormap, rgb: u32) -> GcState<'a,'b> {
        let r = ((rgb >> 16) * 0x101) as u16;
        let g = (((rgb >> 8) & 0xFF) * 0x101) as u16;
        let b = ((rgb & 0xFF) * 0x101) as u16;
        let cookie: xcb::AllocColorCookie = xcb::alloc_color(conn, colormap, r, g, b);
        GcState { cookie, screen }
    }

    fn finalize(self) -> u32 {
        let conn = self.cookie.conn;
        let pixel = self.cookie.get_reply().unwrap().pixel();
        let id = conn.generate_id();
        xcb::create_gc(conn, id, self.screen.root(), &[
            (xcb::GC_FOREGROUND, pixel),
            (xcb::GC_GRAPHICS_EXPOSURES, 0),
        ]);
        id
    }
}

fn rect(x0: i16, x1: i16, y0: i16, y1: i16) -> Option<xcb::Rectangle> {
    if x1 >= x0 && y1 >= y0 {
        Some(xcb::Rectangle::new(x0, y0, (x1 - x0 + 1) as u16, (y1 - y0 + 1) as u16))
    } else {
        None
    }
}

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

fn create_client() -> Result<Client, Error> {
    let options = ClientOptions::NO_START_SERVER /* | ClientOptions::USE_EXACT_NAME */;
    let (client, status) = Client::new("VU meter", options)?;
    if !(status & ClientStatus::NAME_NOT_UNIQUE).is_empty() {
        println!("We are not alone!");
    }
    Ok(client)
}

#[allow(clippy::default_constructed_unit_structs)]
fn setup_ports(client: &Client, num_channels: usize) -> Vec<Port<AudioIn>> {
    (0..num_channels).map(|chan|
        client.register_port(&format!("in_{}", chan + 1), AudioIn::default()).unwrap_or_else(|_| panic!("Failed to register port {}", chan))
    ).collect()
}

fn connect_ports<T, U>(client_name: String, ac: &AsyncClient<T, U>, ports: Vec<String>, num_channels: usize) -> Result<(), Error> {
    let client = ac.as_client();
    ports.iter()
        .map(|arg| {
            let mut s = arg.splitn(2, ':');
            let num: usize = s.next().expect("Missing channel number").parse()
                .unwrap_or_else(|_| panic!("Malformed channel number, expected number in range 1–{}", num_channels));
            let port = s.next().expect("Missing port");
            if num < 1 || num > num_channels {
                panic!("Bad channel number, should be in range 1–{}", num_channels);
            }
            (num, port)
        })
        .for_each(|(channel, port)|
            client.connect_ports_by_name(port, &format!("{}:in_{}", client_name, channel))
                .unwrap_or_else(|e| {
                    eprintln!("Failed to connect port `{}` to channel {}: {:#?}", port, channel, e);
                    eprintln!("Available:");
                    for port in client.ports(None, Some(AudioOut.jack_port_type()), PortFlags::IS_OUTPUT) {
                        eprintln!("  - `{}`", port);
                    }
                    panic!("Bad connection");
                }));
    Ok(())
}

struct ProcessHandlerContext {
    ports: Vec<Port<AudioIn>>,
    vu: Arc<Mutex<Vec<f32>>>,
}

impl ProcessHandlerContext {
    fn new(
        ports: Vec<Port<AudioIn>>,
    ) -> ProcessHandlerContext {
        let num = ports.len();
        let mut vu = Vec::with_capacity(num);
        vu.resize(num, 0f32);
        ProcessHandlerContext {
            ports,
            vu: Arc::new(Mutex::new(vu)),
        }
    }

    fn vu(&self) -> Arc<Mutex<Vec<f32>>> {
        Arc::clone(&self.vu)
    }
}

impl ProcessHandler for ProcessHandlerContext {
    fn process(&mut self, _client: &Client, ps: &ProcessScope) -> Control {
        let mut vu= self.vu.lock().unwrap();
        self.ports.iter().enumerate().for_each(|(i, chan)| {
            let max_of_chan = chan.as_slice(ps).iter().map(|s| s.abs()).max_by(|a, b| a.partial_cmp(b).unwrap()).unwrap();
            vu[i] = vu[i].max(max_of_chan);
        });
        Control::Continue
    }
}

struct NotificationHandlerContext {}

impl NotificationHandler for NotificationHandlerContext {}
