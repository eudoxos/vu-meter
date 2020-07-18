use xcb;
use jack::*;
use clap::{App, Arg};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

fn main() {
    let matches = cli_args().get_matches();
    let channels = matches.value_of("channels").map(|p| p.parse::<u32>().unwrap()).unwrap();

    let client = create_client().expect("Failed to create Jack client");
    let ports = setup_ports(&client, channels);

    let process_handler_context = ProcessHandlerContext::new(
        ports,
    );

    let vu = process_handler_context.vu();

    let notification_handler_context = NotificationHandlerContext { };

    let frame_dur_ms = 1000 * client.buffer_size() / client.sample_rate() as u32;

    let _ac = match client.activate_async(notification_handler_context, process_handler_context) {
        Ok(ac) => ac,
        Err(e) => {
            println!("Failed to activate {:?}", e);
            return;
        }
    };

    let (conn, screen_num) = xcb::Connection::connect(None).unwrap();
    let conn = Arc::new(conn);
    let screen = conn.get_setup().roots().nth(screen_num as usize).unwrap();

    let colormap = screen.default_colormap();
    let gc_cookies = [
        0x000000u32, // background
        0x5DE73D, // meter low
        0xFFFF00, // meter med
        0xFF0000, // meter high
        0x062806, // grid low
        0x282806, // grid med 1
        0x472806, // grid med 2
        0x280F06, // grid high
    ].iter()
        .map(|rgb| [
            ((rgb >> 16) * 0x101) as u16,
            (((rgb >> 8) & 0xFF) * 0x101) as u16,
            ((rgb & 0xFF) * 0x101) as u16
        ])
        .map(|[r, g, b]| xcb::alloc_color(&conn, colormap, r, g, b))
        .collect::<Vec<xcb::AllocColorCookie>>();

    let mut gc = gc_cookies.into_iter()
        .map(|cookie| cookie.get_reply().unwrap().pixel())
        .map(|pixel| {
            let id = conn.generate_id();
            xcb::create_gc(&conn, id, screen.root(), &[
                (xcb::GC_FOREGROUND, pixel),
                (xcb::GC_GRAPHICS_EXPOSURES, 0),
            ]);
            id
        });
    let gc_bg = gc.next().unwrap();
    let gc_meter_low = gc.next().unwrap();
    let gc_meter_med = gc.next().unwrap();
    let gc_meter_high = gc.next().unwrap();
    let gc_grid_low = gc.next().unwrap();
    let gc_grid_med1 = gc.next().unwrap();
    let gc_grid_med2 = gc.next().unwrap();
    let gc_grid_high = gc.next().unwrap();
    assert!(gc.next().is_none());

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
                 xcb::EVENT_MASK_KEY_PRESS |
                 xcb::EVENT_MASK_STRUCTURE_NOTIFY
            ),
        ]
    );
    xcb::map_window(&conn, win);
    xcb::change_property(&conn, xcb::PROP_MODE_REPLACE as u8, win,
                         xcb::ATOM_WM_NAME, xcb::ATOM_STRING, 8, title.as_bytes());

    {
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

    conn.flush();

    loop {
        let event = conn.wait_for_event();
        match event {
            None => { break; }
            Some(event) => {
                let r = event.response_type() & !0x80;
                match r {
                    xcb::EXPOSE => {
                        let ch = {
                            let mut src = vu.lock().unwrap();
                            let copy = src.clone();
                            src.iter_mut().for_each(|i| *i = 0f32);
                            copy
                        };
                        /*
                        let event : &xcb::ExposeEvent = unsafe {
                            xcb::cast_event(&event)
                        };
                        let evt_x0 = event.x();
                        let evt_y0 = event.y();
                        let evt_x1 = evt_x0 + event.width() - 1;
                        let evt_y1 = evt_y0 + event.height() - 1;
                        println!("Expose {},{} - {},{}", evt_x0, evt_y0, evt_x1, evt_y1);
                         */
                        let x: (i16, i16) = (0, win_w as i16 - 1);
                        let y: (i16, i16) = (0, win_h as i16 - 1);

                        let e = ch.len();

                        // bar chart
                        let locations = ch.iter()
                            .enumerate()
                            .map(|(i, level)| {
                                let x0 = interp_i(x.0, x.1, i, e);
                                let x1 = interp_i(x.0, x.1, i + 1, e);
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
                    xcb::KEY_PRESS => {
                        let event: &xcb::KeyPressEvent = unsafe {
                            xcb::cast_event(&event)
                        };
                        println!("Key '{}' pressed", event.detail());
                        //break;
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
    let (client, status) = Client::new("vumeter", options)?;
    if !(status & ClientStatus::NAME_NOT_UNIQUE).is_empty() {
        println!("We are not alone!");
    }
    Ok(client)
}

fn setup_ports(client: &Client, channels: u32) -> Vec<Port<AudioIn>> {
    (0..channels).map(|chan|
        client.register_port(&format!("in_{}", chan), jack::AudioIn::default()).expect(&format!("Failed to register port {}", chan))
    ).collect()
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

impl NotificationHandler for NotificationHandlerContext {
    fn thread_init(&self, _: &Client) {}

    /// Called when the JACK server shuts down the client thread. The function
    /// must be written as if
    /// it were an asynchronous POSIX signal handler --- use only async-safe
    /// functions, and remember
    /// that it is executed from another thread. A typical funcion might set a
    /// flag or write to a
    /// pipe so that the rest of the application knows that the JACK client
    /// thread has shut down.
    fn shutdown(&mut self, _status: ClientStatus, _reason: &str) {}

    /// Called whenever "freewheel" mode is entered or leaving.
    fn freewheel(&mut self, _: &Client, _is_freewheel_enabled: bool) {}

    /// Called whenever the size of the buffer that will be passed to `process`
    /// is about to change.
    fn buffer_size(&mut self, _: &Client, _size: Frames) -> Control {
        Control::Continue
    }

    /// Called whenever the system sample rate changes.
    fn sample_rate(&mut self, _: &Client, _srate: Frames) -> Control {
        Control::Continue
    }

    /// Called whenever a client is registered or unregistered
    fn client_registration(&mut self, _: &Client, _name: &str, _is_registered: bool) {}

    /// Called whenever a port is registered or unregistered
    fn port_registration(&mut self, _: &Client, _port_id: PortId, _is_registered: bool) {}

    /// Called whenever a port is renamed.
    fn port_rename(
        &mut self,
        _: &Client,
        _port_id: PortId,
        _old_name: &str,
        _new_name: &str,
    ) -> Control {
        Control::Continue
    }

    /// Called whenever ports are connected/disconnected to/from each other.
    fn ports_connected(
        &mut self,
        _: &Client,
        _port_id_a: PortId,
        _port_id_b: PortId,
        _are_connected: bool,
    ) {
    }

    /// Called whenever the processing graph is reordered.
    fn graph_reorder(&mut self, _: &Client) -> Control {
        Control::Continue
    }

    /// Called whenever an xrun occurs.
    ///
    /// An xrun is a buffer under or over run, which means some data has been
    /// missed.
    fn xrun(&mut self, _: &Client) -> Control {
        Control::Continue
    }

    /// Called whenever it is necessary to recompute the latencies for some or
    /// all JACK ports.
    ///
    /// It will be called twice each time it is needed, once being passed
    /// `CaptureLatency` and once
    /// with `PlayBackLatency. See managing and determining latency for the
    /// definition of each type
    /// of latency and related functions. TODO: clear up the "see managing and
    /// ..." in the
    /// docstring.
    ///
    /// IMPORTANT: Most JACK clients do NOT need to register a latency callback.
    ///
    /// Clients that meed any of the following conditions do NOT need to
    /// register a latency
    /// callback:
    ///
    /// * have only input ports
    ///
    /// * have only output ports
    ///
    /// * their output is totally unrelated to their input
    ///
    /// * their output is not delayed relative to their input (i.e. data that
    /// arrives in a `process`
    /// is processed and output again in the same callback)
    ///
    /// Clients NOT registering a latency callback MUST also satisfy this
    /// condition
    ///
    /// * have no multiple distinct internal signal pathways
    ///
    /// This means that if your client has more than 1 input and output port,
    /// and considers them
    /// always "correlated" (e.g. as a stereo pair), then there is only 1 (e.g.
    /// stereo) signal
    /// pathway through the client. This would be true, for example, of a
    /// stereo FX rack client that
    /// has a left/right input pair and a left/right output pair.
    ///
    /// However, this is somewhat a matter of perspective. The same FX rack
    /// client could be
    /// connected so that its two input ports were connected to entirely
    /// separate sources. Under
    /// these conditions, the fact that the client does not register a latency
    /// callback MAY result
    /// in port latency values being incorrect.
    ///
    /// Clients that do not meet any of those conditions SHOULD register a
    /// latency callback.
    ///
    /// See the documentation for `jack_port_set_latency_range()` on how the
    /// callback should
    /// operate. Remember that the mode argument given to the latency callback
    /// will need to be
    /// passed into jack_port_set_latency_range()
    fn latency(&mut self, _: &Client, _mode: LatencyType) {}
}

fn cli_args<'a, 'b>() -> App<'a, 'b> {
    App::new("vu-meter")
        .version("1.0")
        .author("Jonas Berlin <xkr47@outerspace.dyndns.org>")
        .about("Jack VU-Meter inspired by cadence-jackmeter")
        .arg(Arg::with_name("channels")
            .short("c")
            .long("channels")
            .value_name("NUM_CHANNELS")
            .help("Sets the number of input channels (default 2)")
            .takes_value(true)
            .default_value("2")
        )
    /*
    .arg(Arg::with_name("WAV")
        .help("WAV file(s) to render")
        .required(true)
        .multiple(true)
    )
    .arg(Arg::with_name("verbose")
        .short("v")
        .long("verbose")
        .help("Enable verbose mode"))
    .arg(Arg::with_name("config")
        .short("c")
        .long("config")
        .value_name("FILE")
        .help("Sets a custom config file")
        .takes_value(true))
    .subcommand(SubCommand::with_name("test")
        .about("controls testing features")
        .version("1.3")
        .author("Someone E. <someone_else@other.com>")
        .arg(Arg::with_name("debug")
            .short("d")
            .help("print debug information verbosely")))
            */
}
