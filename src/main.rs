#[macro_use] 
extern crate serde_derive;
#[macro_use]
extern crate log;
extern crate getopts;

extern crate alsa;
extern crate byteorder;
extern crate bincode;
extern crate serde;
extern crate env_logger;
extern crate rand;

mod packet_layer;
mod audio;

use packet_layer::packet_layer;
use audio::RingBuffer;
use rand::Rng;
use std::io;
use std::io::BufRead;
use std::thread;
use std::collections::HashMap;
use std::sync;
use std::env;
use std::io::Write;
use byteorder::{BigEndian, WriteBytesExt, ReadBytesExt};
use getopts::Options;

fn main() {
    env_logger::init().unwrap();
    let mut rng = rand::thread_rng();


    let args: Vec<String> = env::args().collect();

    let mut opts = Options::new();
    opts.optopt("b", "ring-buffer-size", "set ring-buffer size in bytes", "SIZE");
    opts.optopt("r", "read-bucket-size", "set bucket size in bytes for reads from ring buffer", "SIZE");
    opts.optopt("w", "write-bucket-size", "set bucket size in bytes for writes to ring buffer", "SIZE");
    opts.optopt("s", "spare-size", "set size in bytes of spare area in ring buffer", "SIZE");
    opts.optopt("d", "delay", "set delay in ms", "DELAYMS");
    opts.optopt("a", "audio-device", "set name of audio-device", "NAME");
    opts.optflag("h", "help", "print this help menu");
    let matches = match opts.parse(&args[1..]) {
        Ok(m) => m,
        Err(e) => { panic!("Failed to parse Arguments") }
    };
    if matches.opt_present("h") {
        println!("{}", opts.usage(format!("{}: {}", &*args[0], "audio record and play").as_str()));
        return;
    }
    let ring_buf_len = match matches.opt_str("b") {
        Some(val) => val.parse().unwrap_or_else(|err| panic!("could not parse '{}': {}", args[1], err)),
        None => 1400 * 100
    };
    let read_bucket_len = match matches.opt_str("r") {
        Some(val) => val.parse().unwrap_or_else(|err| panic!("could not parse '{}': {}", args[1], err)),
        None => 10000
    };
    let write_bucket_len = match matches.opt_str("w") {
        Some(val) => val.parse().unwrap_or_else(|err| panic!("could not parse '{}': {}", args[1], err)),
        None => 1400
    };
    let spare_len = match matches.opt_str("s") {
        Some(val) => val.parse().unwrap_or_else(|err| panic!("could not parse '{}': {}", args[1], err)),
        None => 1400 * 20
    };
    let delay = match matches.opt_str("d") {
        Some(val) => val.parse().unwrap_or_else(|err| panic!("could not parse '{}': {}", args[1], err)),
        None => 1000
    };
    let devname = match matches.opt_str("a") {
        Some(val) => val,
        None => "default".to_string()
    };

    //let config = audio::AudioConfig { devname: "plughw:Set", num_channels: 1, sample_rate: 44100 };
    let config = audio::AudioConfig { devname: &devname, num_channels: 1, sample_rate: 44100 };

    let port = 1337;
    let addr = rng.gen();

    let (mut tx,rx) = packet_layer(port,addr).unwrap();

    let buffer_mutex_play = sync::Arc::new(sync::Mutex::new(audio::AudioBuffer::new(ring_buf_len, spare_len)));
    let buffer_mutex_write = buffer_mutex_play.clone();

    //audio::Recorder::spawn_record_thread(&config, 12345, buffer_mutex_write);

    //std::thread::sleep(std::time::Duration::from_millis(delay));
    
    //let mut audio_buffer = audio::AudioBuffer::new(config.buf_len);
    
    thread::spawn(move || {
    	loop {
    	    let (data, _, _) = rx.receive().unwrap();
	    buffer_mutex_write.lock().unwrap().store_data(data);
    	}
    });

    audio::Player::spawn_play_thread(&config, read_bucket_len, buffer_mutex_play);
    //thread::spawn(move || {
    let recorder = audio::Recorder::new(&config, rng.gen()).unwrap();
    recorder.record(write_bucket_len, |data| {tx.send(data); });
    //});
    //loop{
    //    std::thread::sleep(std::time::Duration::from_millis(20000));
    //}

    //std::thread::sleep(std::time::Duration::from_millis(delay));
	
    /*thread::spawn(move || {
        loop {
            let (payload, address, sequence_number) = rx.receive().unwrap();
            println!("{},{}: {}",address, sequence_number, String::from_utf8(payload).unwrap());
        }
    });

    loop {
        let stdin = io::stdin();
        for line in stdin.lock().lines() {
            tx.send(line.unwrap().into_bytes());
        }
    }*/

}
