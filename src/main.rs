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
    opts.optopt("b", "buffer-size", "set buffer size in byte", "SIZE");
    opts.optopt("d", "delay", "set delay in ms", "DELAYMS");
    opts.optflag("h", "help", "print this help menu");
    let matches = match opts.parse(&args[1..]) {
        Ok(m) => m,
        Err(e) => { panic!("Failed to parse Arguments") }
    };
    if matches.opt_present("h") {
        return;
    }
    let buf_len = match matches.opt_str("b") {
        Some(val) => val.parse().unwrap_or_else(|err| panic!("could not parse buffer size '{}': {}", args[1], err)),
        None => 10240
    };
    let delay = match matches.opt_str("d") {
        Some(val) => val.parse().unwrap_or_else(|err| panic!("could not parse delay '{}': {}", args[2], err)),
        None => 1000
    };

    let config = audio::AudioConfig { devname: "default", num_channels: 1, sample_rate: 44100, buf_len: buf_len };

    let port = 1337;
    let addr = rng.gen();

    let (mut tx,rx) = packet_layer(port,addr).unwrap();

	let buffer_mutex_play = sync::Arc::new(sync::Mutex::new(audio::AudioBuffer::new(config.buf_len)));
    let buffer_mutex_write = buffer_mutex_play.clone();

    //audio::Recorder::spawn_record_thread(&config, 12345, buffer_mutex_write);

    //std::thread::sleep(std::time::Duration::from_millis(delay));
    
    //let mut audio_buffer = audio::AudioBuffer::new(config.buf_len);
    
    thread::spawn(move || {
    	let mut known_senders = HashMap::new();
    	loop {
    		let ((sender, data), addr, _) = rx.receive().unwrap();
    		if let None = known_senders.get(&sender) {
	            let ring = RingBuffer::new(10);
    			buffer_mutex_write.lock().unwrap().add_client(sender, ring);
    			known_senders.insert(sender, ());
    		}
			buffer_mutex_write.lock().unwrap().store_data(sender, data);
    	}
    });

    let recorder = audio::Recorder::new(&config).unwrap();
    let sender = rng.gen();
	thread::spawn(move || {
		recorder.record(|data| {tx.send((sender, data)); });
	});

	std::thread::sleep(std::time::Duration::from_millis(delay));
    audio::Player::spawn_play_thread(&config, buffer_mutex_play);
	loop{
		std::thread::sleep(std::time::Duration::from_millis(20000));
    }
	
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
