use std::{fs::OpenOptions, io::Write, net::{SocketAddr, TcpListener}, path::PathBuf, sync::{atomic::{AtomicBool, Ordering}, Arc}, thread::{self, spawn}, time::Duration, vec};

use chrono::{Local, NaiveDateTime};
use clap::Parser;

use dataforge::{read_df_message_sync, write_df_message_sync};
use eframe::{egui::{self, mutex::Mutex}, NativeOptions};
use egui_plot::{Legend, Plot, PlotPoints, Points};
use log::{debug, error, info, warn};
use numass::{NumassMeta, Reply};

#[cfg(not(feature = "virtual"))]
const BOARD_NUM: i32 = 24; 

#[cfg(not(feature = "virtual"))]
extern "C" {
    fn ibdev(board: i32, pad: i32, sad: i32, timo: i32, send_eoi: i32, eosmode: i32) -> i32;
    // fn ibwrt(board: i32, buf: *const u8, cnt: i32) -> i32;
    fn ibrd(board: i32, buf: *mut u8, cnt: i32) -> i32;
    fn ibwrt(board: i32, buf: *const u8, cnt: i32) -> i32;
}

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(short, long)]
    data_root: Option<PathBuf>,
    #[arg(short, long, default_value_t = 1000)]
    number_elements_to_plot: usize, 

    /// A port for control acquision from online system.
    #[arg(long, default_value_t = 8080)]
    service_port: u16,
}

struct DisplayApp {
    buffer: Arc<Mutex<Vec<f32>>>,
}


impl eframe::App for DisplayApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {

        ctx.request_repaint_after(Duration::from_secs(1));

        egui::CentralPanel::default().show(ctx, |ui| {

            let my_plot = Plot::new("I Plot")
                .legend(Legend::default())
                .y_axis_formatter( |mark, _,| { format!("{:+e}", mark.value) })
            ;
            let points = {
                let data = self.buffer.lock().clone();
                data.into_iter().enumerate().map(|(idx, val)| {
                    [idx as f64, val as f64]
                }).collect::<Vec<_>>()
            };

            my_plot.show(ui, |plot_ui| {
                plot_ui.points(Points::new(PlotPoints::from(points)).radius(3.0).name("curve"));
            });
        });
    }
}

type Timeseries = Vec<(NaiveDateTime, String)>;

fn service(buffer: Arc<Mutex<Option<Timeseries>>>, port: u16) {

    let listener = TcpListener::bind(SocketAddr::new([0,0,0,0].into(), port)).unwrap();
    info!("faradey-server works on 0.0.0.0:{}", port);

    let mut current_running: Option<Arc<AtomicBool>> = None;

    loop {
        let (mut socket, _) = listener.accept().unwrap();

        if current_running.is_some() {
            info!("new connection. aborting previous one.");
            if let Some(val) = current_running { val.store(false, Ordering::SeqCst) }
        }
        current_running = Some(Arc::new(AtomicBool::new(true)));

        let running_local = Arc::clone(current_running.as_ref().unwrap());
        let buffer = Arc::clone(&buffer);
        thread::spawn(move || {
            loop {
                while running_local.load(Ordering::SeqCst) {
                    let msg = read_df_message_sync(&mut socket).expect("catch IO error on receiving DF message");
                    info!("received message: {msg:?}");

                    match msg.meta {
                        NumassMeta::Command(command) => match command {
                            numass::Command::Init => {
                                write_df_message_sync(
                                    &mut socket,
                                    NumassMeta::Reply(numass::Reply::Init {
                                        status: numass::ReplyStatus::Ok,
                                        reseted: false,
                                    }),
                                    None,
                                )
                                .expect("catch IO error on sending DF message");
                            }
                            numass::Command::AcquirePoint {
                                split: _,
                                acquisition_time,
                                path: _,
                                external_meta,
                            } => {
                                let start_time = Local::now().naive_local();
                                {
                                    let mut lock = buffer.lock();
                                    *lock = Some(vec![]);
                                }

                                thread::sleep(Duration::from_secs_f32(acquisition_time));
                                let data = buffer.lock().clone().unwrap();

                                {
                                    let mut lock = buffer.lock();
                                    *lock = None;
                                }
                                let end_time = Local::now().naive_local();

                                debug!("to online: {data:?}");

                                let mut table = "timestamp\tvalue\n".to_string();
                                for (timestamp, value) in data {
                                    table.push_str(&format!("{}\t{}\n", timestamp.and_local_timezone(Local).unwrap().to_rfc3339(), value));
                                }

                                write_df_message_sync(&mut socket, 
                                    Reply::AcquirePoint { 
                                        acquisition_time, 
                                        start_time, 
                                        end_time, 
                                        external_meta, 
                                        config: None, 
                                        zero_suppression: None, 
                                        status: numass::ReplyStatus::Ok 
                                    },
                                    Some(table.as_bytes().to_owned())
                                ).expect("catch IO error on sending DF message");
                            }
                        },
                        _ => {
                            write_df_message_sync(
                                &mut socket,
                                NumassMeta::Reply(numass::Reply::Error {
                                    error_code: numass::ErrorType::UnknownMessageError,
                                    description: "tqdc-server doesn't handles anything but commands"
                                        .to_string(),
                                }),
                                None,
                            )
                            .expect("catch IO error on sending DF message");
                        }
                    }
                }
            }
        });
    }
}

fn main() {

    env_logger::init();

    #[cfg(feature = "virtual")] {
        warn!("Running in virtual mode");
    }

    let args = Args::parse();

    let plot_buffer = Arc::new(Mutex::new(
        Vec::with_capacity(args.number_elements_to_plot * 2)));
    let point_buffer: Arc<Mutex<Option<Timeseries>>> = Arc::new(Mutex::new(None));

    {
        let plot_buffer = Arc::clone(&plot_buffer);
        let point_buffer = Arc::clone(&point_buffer);

        spawn(move || {
            let mut data_file = {
                let data_root = args.data_root.unwrap_or(PathBuf::from("data"));
        
                let now = Local::now().naive_local();
        
                let today_root = data_root.join(format!("{}", now.format("%y-%m-%d")));
        
                std::fs::create_dir_all(&today_root).unwrap();
        

                #[cfg(feature = "virtual")]
                let current_file = today_root.join(format!("{}-virtual.tsv", now.format("%H-%M")));
                #[cfg(not(feature = "virtual"))]
                let current_file = today_root.join(format!("{}.tsv", now.format("%H-%M")));

                info!("writing to {current_file:?}");
                OpenOptions::new()
                    .append(true)
                    .create(true)
                    .open(current_file)
                    .unwrap()
            };
            
            // let mut asteriks = None;
    
            // pub const T10us: c_int = 1;
            // pub const T30us: c_int = 2;
            // pub const T100us: c_int = 3;
            // pub const T300us: c_int = 4;
            // pub const T1ms: c_int = 5;
            // pub const T3ms: c_int = 6;
            // pub const T10ms: c_int = 7;
            // pub const T30ms: c_int = 8;
            // pub const T100ms: c_int = 9;
            // pub const T300ms: c_int = 10;
            // pub const T1s: c_int = 11;
            // pub const T3s: c_int = 12;
            // pub const T10s: c_int = 13;
            // pub const T30s: c_int = 14;
            // pub const T100s: c_int = 15;
            // pub const T300s: c_int = 16;
            // pub const T1000s: c_int = 17;

            #[cfg(not(feature = "virtual"))]
            let ud = unsafe {
                ibdev(0, BOARD_NUM, 0, 15, 1, 0)
            };

            
            loop {
                let parts = {
                    #[cfg(not(feature = "virtual"))] {
                        let cmd = ":READ?";
                        unsafe {
                            ibwrt(ud, cmd.as_ptr(), cmd.len() as i32);
                        }

                        let out = {
                            unsafe {
                                let mut buf = [0u8; 4096 * 20];
                                ibrd(ud, buf.as_mut_ptr(), buf.len() as i32);
                                buf
                            }
                        };

                        let out = String::from_utf8(out.to_vec()).unwrap();
                        let ans = out.trim_end_matches(char::from(0)).to_owned();
                
                        let parts = ans.split(',').map(|part| part.to_string()).collect::<Vec<_>>();
                        parts
                    }

                    #[cfg(feature = "virtual")] {
                        // wait 0.63 seconds
                        std::thread::sleep(Duration::from_secs_f32(0.63)); // 1/1.6 seconds
                        let generated_point = format!("{:+.6E}", rand::random_range(1e-13..3e-13));
                        vec![generated_point]
                    }

                };
                let timestamp = Local::now().naive_local();

                if !parts.is_empty() {
                    if !parts.is_empty() {
                        debug!("{}", &parts.last().unwrap());
                    } else {
                        warn!("empty message after pop asterics")
                    }
            
                    for (idx, voltage) in parts.iter().enumerate() {
                        if idx == 0 {
                            data_file.write_all(
                                timestamp.and_local_timezone(Local).unwrap().to_rfc3339().as_bytes()
                            ).unwrap();
                        }
                        data_file.write_all(b"\t").unwrap();
                        data_file.write_all(voltage.as_bytes()).unwrap();
                        data_file.write_all(b"\n").unwrap();
                    }
                    data_file.flush().unwrap();

                    // fill point data if acqusition is running (point_buffer != None)
                    {
                        if let Some(point_buffer) = point_buffer.lock().as_mut() {
                            for raw_value in &parts {
                                point_buffer.push((timestamp, raw_value.to_owned()));   
                            }
                        }
                    }
    
                    {
                        let mut buffer = plot_buffer.lock();
                        let n_to_plot = args.number_elements_to_plot;
    
                        if buffer.len() > n_to_plot * 2 {
                            *buffer = buffer[n_to_plot..].to_vec();
                        }

                        for raw_value in parts {
                            if let Ok(value) = raw_value.parse::<f32>() {
                                buffer.push(value);
                            } else {
                                error!("error parsing raw value: {raw_value:?}")
                            }
                        }
                    }
                } else {
                    warn!("empty message")
                }
            }
        });
    }

    {
        let point_buffer = Arc::clone(&point_buffer);
        thread::spawn(move || {
            service(point_buffer, args.service_port);
        });
    }
    
    eframe::run_native(
        "Keithley 6430",
        NativeOptions::default(),
        Box::new(move |_| {
            Ok(Box::<DisplayApp>::new(DisplayApp { 
                buffer: plot_buffer
            }))
        }),
    ).unwrap();
    
}
