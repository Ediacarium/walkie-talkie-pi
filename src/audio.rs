use std::collections::HashMap;
use std::thread;
use std::sync;
use std::ffi::CString;
use std::clone::Clone;
use std;
use alsa::{Direction, ValueOr};
use alsa::pcm::{PCM, HwParams, Format, Access, Frames};
use bincode::{serialize, deserialize};

#[derive(Deserialize, Serialize, Clone)]
pub struct AudioData {
    pub buf: Vec<i16>,
    pub id: usize,
}

pub struct RingBuffer {
    ring: Vec<AudioData>,
    ring_size: usize,
    next: usize,
    max: usize,
}

impl RingBuffer {
    pub fn new(ring_size: usize) -> RingBuffer {
        let mut ring: Vec<AudioData> = Vec::new();
        for _ in 0..ring_size {
            ring.push(AudioData { buf: vec![], id: 0});
        }
        RingBuffer { ring: ring, ring_size: ring_size, max: 0, next: 1 }
    }

    fn visualize_ring(&self) {
        if self.ring[self.next % self.ring_size].id == self.next {
            print!("{}", self.next);
        }
        else {
            print!("({})", self.next);
        }            
        for i in 1..self.ring_size {
            let index = (i + self.next) % self.ring_size;
            if i + self.next < self.max {
                if self.ring[index].id == i + self.next {
                    print!("+");
                }
                else {
                    print!(".");
                }
            }
            else if i + self.next == self.max {
                print!("<{}>", self.max);
            }
            else {
                print!(".");
            }
        }
        println!("");
    }
    
    pub fn get_next(&mut self) -> Option<&AudioData> {
        // At this point we may update next towards max in order to reduce delay
        print!("ring: "); self.visualize_ring();
        let index = self.next % self.ring_size;
        let result = if self.ring[index].id == self.next {
            println!("get_next(): return id {} at index {} with max {}", self.next, index, self.max);
            Some(&self.ring[index])
        }
        else {
            println!("get_next(): no data found for id {} with max {}, returning None", self.next, self.max);
            None
        };
        self.next = self.next + 1;  // TODO: What if we get beyond max?
        result
    }

    fn store_data(&mut self, data: AudioData) {
        if data.id < self.next {
            println!("Received id {} is too old. Ignoring", data.id);
            return;
        }
        let index = data.id % self.ring_size;
        print!("Receiving id {}, will be filled at index {}.", data.id, index);
        if data.id > self.max {
            let delta = data.id - self.next;
            if delta >= self.ring_size {
                let oldnext = self.next;
                self.next = data.id - self.ring_size + 1;
                print!(" Beyond ring capactiy, setting next from {} to {}.", oldnext, self.next);
            }
            print!(" Advancing max to {}, next is {}.", data.id, self.next);
            self.max = data.id;
        }
        println!(" Spare capacity is {}", self.ring_size - (self.max - self.next) - 1);
        self.ring[index] = data;
     }


}

// ========================================

pub struct AudioBuffer {
    buf_size: usize,
    rings: HashMap<usize, RingBuffer>,
}

impl AudioBuffer {
    // buf_size is needed in order to create silence and temp buffer
    pub fn new(buf_size: usize) -> AudioBuffer {
        AudioBuffer {buf_size: buf_size, rings: HashMap::new()}
    }

    pub fn add_client(&mut self, client_id: usize, ring: RingBuffer) {
        self.rings.insert(client_id, ring);
    }

    #[allow(dead_code)]
    pub fn rm_client(&mut self, client_id: usize) {
        match self.rings.remove(&client_id) {
            Some(_) => println!("rm_client: Successfully removed {}", client_id),
            None => println!("rm_client: called for {} which does not exist!", client_id)
        }
    }
    
    pub fn store_data(&mut self, client_id: usize, data: AudioData) -> bool {
        match self.rings.get_mut(&client_id) {
            Some(buffer) => buffer.store_data(data),
            None => {
                println!("store_data() called for non-existing client id {}", client_id);
                return false
            }
        };
        true
    }

    pub fn get_next(&mut self) -> Vec<i16> {
        let mut vec = vec![0_i16;self.buf_size];
        let mut count = 0;
        // Adding all buffers:
        for (_, buffer) in &mut self.rings {
            //println!("Calling get_next() for client {}:", id);
            let data = match buffer.get_next() {
                Some(data) => data,
                None => {
                    println!(" No data.");
                    continue
                }
            };
            //println!(" Got data for client {}", id);
            count = count + 1;
            for (pos, val) in data.buf.iter().enumerate() {
                //println!("Adding {} to {} at pos {}", *val, vec[pos], pos);
                vec[pos] += *val;
            }
            //println!("done");
        }
        // Scaling:
        if count > 1 {
            for data in &mut vec {
                *data = *data / count;
            }
        }
        vec
    }

}

// ========================================

pub struct AudioConfig<'a> {
    pub devname: &'a str,
    pub num_channels: u32,
    pub sample_rate: u32,
    pub buf_len: usize,
}

// ========================================

#[allow(dead_code)]
pub struct Player {
    pcm: PCM,
    sample_rate: u32,  // currently not used
}

impl Player {
    pub fn new(config: &AudioConfig) -> Result<Player, Box<std::error::Error>> {
        let cs = CString::new(config.devname)?;
        let pcm = PCM::open(&*cs, Direction::Playback, false)?;
        let sample_rate;
        {
            let hwp = HwParams::any(&pcm)?;
            hwp.set_channels(config.num_channels)?;
            sample_rate = hwp.set_rate_near(config.sample_rate, ValueOr::Nearest)?;
            if sample_rate != config.sample_rate {
                println!("Sample rate was changed from {} to {}", config.sample_rate, sample_rate);
            }
            hwp.set_format(Format::s16())?;
            hwp.set_access(Access::RWInterleaved)?;
            println!("Buffersize: {:?}", hwp.get_buffer_size());
            pcm.hw_params(&hwp)?;
        };
        pcm.prepare()?;
        Ok(Player { pcm: pcm, sample_rate: sample_rate } )
    }

    pub fn get_remain(&self) -> Frames {
        let total = match self.pcm.status() {
            Ok(val) => val.get_avail_max(),
            Err(e) => {
                println!("******ERROR: get_avail_max() returned error {}", e);
                self.pcm.prepare().unwrap();
                return 0  // TODO: Is this OK?
            }
        };
        if total == 0 {
            return 0
        }
        let avail = match self.pcm.avail_update() {
            Ok(val) => val,
            Err(e) => {
                println!("******ERROR: avail_update returned error {}, call prepare", e);
                self.pcm.prepare().unwrap();
                return 0
            }
        };
        println!("Current audio buffer status: total: {}, avail: {}, total-avail {}", total, avail, total-avail);
        total - avail
    }
    
    pub fn play(&self, data: Vec<i16>) {
        let io = self.pcm.io_i16().unwrap();
        println!("calling writei");
        io.writei(&data[..]).unwrap();
    }

    pub fn spawn_play_thread(config: &AudioConfig, buffer_mutex_play: sync::Arc<sync::Mutex<AudioBuffer>>) {
        println!("Spawning play thread");
        let player = Player::new(config).unwrap();
        let delay = 900.0 * (config.buf_len as f32/ config.sample_rate as f32 );
        let threshold = (1.5 * config.buf_len as f32) as Frames;
        println!("delay {}, threshold {}", delay as u64, threshold);
        thread::spawn(move || {
            loop {
                while player.get_remain() < threshold {
                    println!("Player needs more data, thus calling get_next() and play()");
                    let data = buffer_mutex_play.lock().unwrap().get_next();
                    player.play(data);
                }
                // TODO: Adapt sleep time
                println!("Ready to sleep");
                
                std::thread::sleep(std::time::Duration::from_millis(delay as u64));
            }
        });
    }
    
}

// ========================================

#[allow(dead_code)]
pub struct Recorder {
    pcm: PCM,
    sample_rate: u32,  // currently not used
    buf_len: usize,
}

impl Recorder {

    pub fn new(config: &AudioConfig) -> Result<Recorder, Box<std::error::Error>> {
        let cs = CString::new(config.devname)?;
        let pcm = PCM::open(&*cs, Direction::Capture, false)?;
        let sample_rate;
        // The following block is needed to prevent borrow compile error because of hwp
        {
            let hwp = HwParams::any(&pcm)?;
            hwp.set_channels(config.num_channels)?;
            sample_rate = hwp.set_rate_near(config.sample_rate, ValueOr::Nearest)?;
            if sample_rate != config.sample_rate {
                println!("Sample rate was changed from {} to {}", config.sample_rate, sample_rate);
            }
            hwp.set_format(Format::s16())?;
            hwp.set_access(Access::RWInterleaved)?;
            pcm.hw_params(&hwp)?;
        }
        Ok(Recorder { pcm : pcm, sample_rate : sample_rate, buf_len : config.buf_len })
    }

    // TODO: To allow mut code in closure, we have to declare F here as FnMut, not Fn. Is this OK?
    pub fn record<F>(& self, mut callback: F) -> Result<(), Box<std::error::Error>>
        where F : FnMut(AudioData) -> ()
    {
        println!("record() start");
        self.pcm.prepare()?;
        let io = self.pcm.io_i16()?;
        let mut i = 0;
        loop {
            i = i + 1;
            let mut data = AudioData{ buf: vec![0_i16; self.buf_len], id: i };
            io.readi(&mut data.buf[..])?;
            callback(data);
        }
    }

    pub fn spawn_record_thread(config: &AudioConfig, client_id: usize, buffer_mutex_write: sync::Arc<sync::Mutex<AudioBuffer>>) {
        println!("Spawning record thread");
        let recorder = Recorder::new(config).unwrap();
        thread::spawn(move || {
            let ring = RingBuffer::new(120);
            buffer_mutex_write.lock().unwrap().add_client(client_id, ring);
            recorder.record(|data| {
                buffer_mutex_write.lock().unwrap().store_data(client_id, data);
            }).unwrap();
        });
    }
}
