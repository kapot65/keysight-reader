use std::{fs::OpenOptions, io::Write, path::PathBuf, sync::{Arc, Mutex}, thread::spawn, time::Duration, vec};

use chrono::Local;
use clap::Parser;

use eframe::{egui, NativeOptions};
use egui_plot::{Legend, Plot, PlotPoints, Points};

const BOARD_NUM: i32 = 24; 

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
}

struct DisplayApp {
    buffer: Arc<Mutex<Vec<f32>>>,
    n_to_plot: usize,
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
                let data = self.buffer.lock().unwrap().clone();
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

fn main() {

    let args = Args::parse();

    let buffer = Arc::new(Mutex::new(
        Vec::with_capacity(args.number_elements_to_plot * 2)));
    let n_to_plot = args.number_elements_to_plot;

    {
        let buffer = Arc::clone(&buffer);
        spawn(move || {
            let mut data_file = {
                let data_root = args.data_root.unwrap_or(PathBuf::from("data"));
        
                let now = Local::now().naive_local();
        
                let today_root = data_root.join(format!("{}", now.format("%d-%m-%y")));
        
                std::fs::create_dir_all(&today_root).unwrap();
        
                let current_file = today_root.join(format!("{}.tsv", now.format("%H-%M")));
                println!("writing to {current_file:?}");
                OpenOptions::new()
                    .append(true)
                    .create(true)
                    .open(today_root.join(format!("{}.tsv", now.format("%H-%M"))))
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

            let ud = unsafe {
                ibdev(0, BOARD_NUM, 0, 15, 1, 0)
            }; 
        
            loop {

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
        
                // let ans = if let Some(ast) = &asteriks {
                //     format!("{ast}{ans}")
                // } else {
                //     ans
                // };
                
                // let cropped = !ans.ends_with("\r\n");
                // let mut parts = ans.split("\r\n")
                //     .filter(|part| !part.is_empty())
                //     .map(|part| part.trim())
                //     .collect::<Vec<_>>();

                let parts = vec![ans.split(',').collect::<Vec<_>>()[1]];

                if !parts.is_empty() {
                    // if cropped {
                    //     asteriks = Some((*parts.last().unwrap()).to_owned());
                    //     parts.pop();
                    // } else {
                    //     asteriks = None
                    // }
            
                    if !parts.is_empty() {
                        let now = Local::now().naive_local();
                        println!("{} {}", now.format("%H:%M:%S"), &parts.last().unwrap());
                    } else {
                        println!("empty message after pop asterics")
                    }
            
                    for (idx, voltage) in parts.iter().enumerate() {
                            data_file.write_all(voltage.as_bytes()).unwrap();
                            data_file.write_all(b"\t").unwrap();
                        if idx == 0 {
                            let now: chrono::NaiveDateTime = Local::now().naive_local();
                            data_file.write_all(now.and_local_timezone(Local).unwrap().to_rfc3339().as_bytes()
                                // now.format("%H:%M:%S").to_string().as_bytes()
                            ).unwrap();
                        }
                        data_file.write_all(b"\n").unwrap();
                    }
                    data_file.flush().unwrap();
    
                    {
                        let mut buffer = buffer.lock().unwrap();
                        let n_to_plot = args.number_elements_to_plot;
    
                        if buffer.len() > n_to_plot * 2 {
                            *buffer = buffer[n_to_plot..].to_vec();
                        }

                        for raw_value in parts {
                            if let Ok(value) = raw_value.parse::<f32>() {
                                buffer.push(value);
                            } else {
                                println!("error parsing raw value: {raw_value:?}")
                            }
                        }
                    }
                } else {
                    println!("empty message")
                }
            }
        });
    }

    eframe::run_native(
        "KeySight",
        NativeOptions::default(),
        Box::new(move |_| {
            Ok(Box::<DisplayApp>::new(DisplayApp { 
                buffer,
                n_to_plot
            }))
        }),
    ).unwrap();
    
}
