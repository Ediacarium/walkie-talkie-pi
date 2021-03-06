extern crate rand;

use rand::Rng;
use std::collections::HashMap;
use std::thread;
use std::sync;
use std::ffi::CString;
use std;
use alsa::{Direction, ValueOr};
use alsa::pcm::{PCM, HwParams, Format, Access, Frames};
use std::time::Instant;


#[derive(Clone, Serialize, Deserialize)]
pub struct AudioData {
    pub client_id: u16,
    pub pos: u64,    // pos must be > 0 (because position 0 is considered to be initial (unset) value
    pub data: Vec<i16>,
}

pub struct RingBuffer {
    buf: Vec<i16>,
    next: u64,  // points to first element to read next, i.e. buf[next] was not yet read
    min: u64,   // points to the first filled element as long as next == 0
    max: u64,   // points to the last filled element, i.e. buf[max] is set
    spare: u64, // number of samples to always keep in buffer
    idle_threshold: u32,  // time in ms until a buffer is marked as idle
    last_update: Instant,
}

enum PeekState {
    Ignore,
    Avail(u64),
    Idle,
    None
}

impl RingBuffer {
    pub fn new(buf_len: u32, spare: u64, idle_threshold: u32) -> RingBuffer {
        assert!(spare < buf_len as u64);
        RingBuffer { buf: vec![0_i16;buf_len as usize], max: 0, next: 0, spare: spare, min: 0, idle_threshold: idle_threshold, last_update: Instant::now()}
    }

    fn debug_print(&self, prefix: &str) {
        trace!("ringbuffer {}: next {}/{} max {}/{} spare {} overhead {} len {} min {}/{} last_update: {:?}", prefix, self.next, self.next % self.buf.len() as u64, self.max, self.max % self.buf.len() as u64, self.spare, self.max - self.next + 1, self.buf.len(), self.min, self.min % self.buf.len() as u64, self.last_update.elapsed());
    }

    pub fn peek(&self, target_len: u64) -> PeekState {
        self.debug_print("peek: ");  // DEBUG
        let buf_len = self.buf.len() as u64;
        // TODO: Combine if clauses (added to have debug output only)
        if self.max == 0 {
            trace!("no data yet added. Return Ignore.");
            return PeekState::Ignore;
        }
        let elapsed = self.last_update.elapsed().as_secs()*1000 + (self.last_update.elapsed().subsec_nanos()/1000000) as u64;
        trace!("elapsed= {}", elapsed);
        if elapsed > self.idle_threshold as u64 {
            trace!("This buffer is idle, elapsed is {}", elapsed);
            return PeekState::Idle;
        }
        if (self.next == 0) && (self.min > 0) && (self.max - self.min) < self.spare {
            trace!("not enough data added yet. Return Ignore.");
            return PeekState::Ignore;
        }
        if (self.next == 0) && (self.max - self.min < target_len) {
            trace!("buffer has only {}, but need {}, return None", self.max - self.min, target_len);
            return PeekState::Ignore;
        }
        if (self.next != 0) && (self.max - self.next < target_len)  {
            trace!("buffer has only {}, but need {}, return None", self.max - self.next, target_len);
            return PeekState::None;
        }
        let avail = if self.next == 0 {self.max - self.min} else {self.max - self.next};
        trace!("we have data, return Avail({})", avail);
        PeekState::Avail(avail)
    }

    pub fn get_next(&mut self, target_len: u64) -> Option<Vec<i16>> {
        assert!(self.spare + target_len < self.buf.len() as u64);
        self.debug_print("get_next: ");  // DEBUG
        let buf_len = self.buf.len() as u64;
        // TODO: Combine if clauses (added to have debug output only)
        if self.max == 0 {
            trace!("no data yet added. Return None.");
            return None;
        }
        if (self.next == 0) && (self.min > 0) && (self.max - self.min) < self.spare {
            trace!("not enough data added yet. Return None.");
            return None;
        }
        if (self.next == 0) && (self.max - self.min < target_len) {
            trace!("buffer has only {}, but need {}, return None", self.max - self.min, target_len);
            return None;
        }
        if (self.next != 0) && (self.max - self.next < target_len)  {
            trace!("buffer has only {}, but need {}, return None", self.max - self.next, target_len);
            return None;
        }
        if self.next == 0 {
            // first call -> set self.next to some sensitive value
            assert!(self.max - self.min  > self.spare);
            self.next = self.min;            
            trace!("First call, setting next to {}", self.next);
        }
        let mut result = vec![0_i16;target_len as usize];
        let start = (self.next % buf_len) as usize;
        let mut end_excl = ((self.next + target_len) % buf_len) as usize;  // buf[end_excl] is the first element not to be returned.
        if end_excl == 0 {
            end_excl = (buf_len + 1) as usize;
        }
        let zero = vec![0_i16;target_len as usize];
        if start < end_excl {
            // current bucket does not exceed buf size, thus we may copy in one piece:
            trace!("get_next(): copy1 [0..{}] <- [{}..{}]", target_len, start, end_excl);
            &result[0..target_len as usize].copy_from_slice(&self.buf[start..end_excl]);  // note: start..end excludes the element at end_excl
            &self.buf[start..end_excl].copy_from_slice(&zero[0..target_len as usize]);  // note: start..end excludes the element at end_excl
        }
        else {
            // current bucket exceeds buf size, thus copy in two pieces:
            let len_first_part = buf_len as usize - start;
            trace!("get_next(): copy2 [0..{}] <- [{}..{}]", len_first_part, start, buf_len);
            trace!("get_next(): copy2 [{}..{}] <- [0..{}]", len_first_part, target_len, end_excl);
            &result[0..len_first_part].copy_from_slice(&self.buf[start..buf_len as usize]);
            &self.buf[start..buf_len as usize].copy_from_slice(&zero[0..len_first_part]);
            &result[len_first_part..target_len as usize].copy_from_slice(&self.buf[0..end_excl]);
            &self.buf[0..end_excl].copy_from_slice(&zero[len_first_part..target_len as usize]);
        }
        self.next = self.next + target_len;
        Some(result)

    }        
//*/    pub fn get_next(&mut self, target_len: u64) -> Option<Vec<i16>> {
//*/        assert!(self.spare + target_len < self.buf.len() as u64);
//*/        self.debug_print();  // DEBUG
//*/        let buf_len = self.buf.len() as u64;
//*/        // TODO: Combine if clauses (added to have debug output only)
//*/        if self.max == 0 {
//*/            trace!("no data yet added. Return None.");
//*/            return None;
//*/        }
//*/        if self.max < self.spare {
//*/            trace!("not enough data added yet. Return None.");
//*/            return None;
//*/        }
//*/        if self.next == 0 {
//*/            // first call -> set self.next to some sensitive value
//*///            self.next = if self.max < buf_len {
//*///                1
//*///            }
//*///            else {
//*///                self.max - buf_len + 1
//*///            };
//*/
//*/            // option 2:
//*///                       self.next = if self.max > self.spare + target_len {
//*///                self.max - self.spare - target_len + 1
//*///            }
//*///            else {
//*///                1  // TODO
//*///            };
//*/            // option 3:
//*/            if self.min > self.max - self.spare {
//*/                self.next = self.min;
//*/                trace!("First call, adjusting next to {}", self.next);
//*/            }
//*/            else {
//*/                //self.next = self.min;
//*/                trace!("First call but not enough space");
//*/            }
//*/            
//*/        }
//*/        if self.next > self.max - self.spare {
//*/            trace!("not enough spare data. Return None.");
//*/            return None;
//*/        }
//*/        let max_avail = ((self.max - self.spare) + 1) - self.next;   // Do not change computation order to prevent underflow!
//*/        let len = if max_avail < target_len {
//*/            trace!("Reducing length of result vector from {} to {}", target_len, max_avail);
//*/            return None; // TILT
//*/            //max_avail
//*/        }
//*/        else {
//*/            target_len
//*/        };
//*/        assert!(len > 0);
//*/        //let mut result = Vec::<i16>::with_capacity(len as usize);
//*/        let mut result = vec![0_i16;len as usize];
//*/        let start = (self.next % buf_len) as usize;
//*/        let mut end_excl = ((self.next + len) % buf_len) as usize;  // buf[end_excl] is the first element not to be returned.
//*/        if end_excl == 0 {
//*/            end_excl = (buf_len + 1) as usize;
//*/        }
//*/        if start < end_excl {
//*/            // current bucket does not exceed buf size, thus we may copy in one piece:
//*/            trace!("get_next(): copy1 [0..{}] <- [{}..{}]", len, start, end_excl);
//*/            &result[0..len as usize].copy_from_slice(&self.buf[start..end_excl]);  // note: start..end excludes the element at end_excl
//*/        }
//*/        else {
//*/            // current bucket exceeds buf size, thus copy in two pieces:
//*/            let len_first_part = buf_len as usize - start;
//*/            trace!("get_next(): copy2 [0..{}] <- [{}..{}]", len_first_part, start, buf_len);
//*/            trace!("get_next(): copy2 [{}..{}] <- [0..{}]", len_first_part, len, end_excl);
//*/            &result[0..len_first_part].copy_from_slice(&self.buf[start..buf_len as usize]);
//*/            &result[len_first_part..len as usize].copy_from_slice(&self.buf[0..end_excl]);
//*/        }
//*/        self.next = self.next + len;
//*/        Some(result)
//*/    }
//*/
    fn store_data(&mut self, data: AudioData) -> Result<Option<()>, String> {
        let buf_len = self.buf.len() as u64;
        trace!("store data from client {} at pos {} of len {}", data.client_id, data.pos, data.data.len());
        assert!(data.data.len() < buf_len as usize);
        if (self.next > 0) && (self.next >= data.pos + data.data.len() as u64) {
            trace!("data with pos {} and length {} is beyond next at {}", data.pos, data.data.len(), self.next);
            return Ok(None);
        }
        if (self.max > 0) && (data.pos <= self.max) && (data.pos + data.data.len() as u64 > self.max) {
            trace!("error: data with pos {} and length {} crosses max at {}", data.pos, data.data.len(), self.max);
            return Err("mismatch data pos and len: Crosses max.".to_string());
        }
        let len = data.data.len();
        let start = (data.pos % buf_len) as usize;
        let mut end_excl = ((data.pos + data.data.len() as u64) % buf_len) as usize;
        if end_excl == 0 {
            end_excl = (buf_len + 1) as usize;
        }
        trace!("store_data(): start {}, end_excl {}", start, end_excl);
        if start < end_excl {
            // current bucket does not exceed buf size, thus we may copy in one piece:
            trace!("store_data(): copy1 [{}..{}] <- [0..{}]", start, end_excl, len);
            &self.buf[start..end_excl].copy_from_slice(&data.data[0..len as usize]);
        }
        else {
            // current bucket exceeds buf size, thus copy in two pieces:
            let len_first_part = (buf_len - start as u64) as usize;
            trace!("store_data(): copy2 [{}..{}] <- [0..{}]", start, buf_len, len_first_part);
            trace!("store_data(): copy2 [0..{}] <- [{}..{}]", end_excl, len_first_part, len);
            &self.buf[start..buf_len as usize].copy_from_slice(&data.data[0..len_first_part]);
            &self.buf[0..end_excl].copy_from_slice(&data.data[len_first_part..len as usize]);
        }
        if data.pos > self.max {  // Note: as data does not cross max (see check above), this condition is sufficient
            self.max = data.pos + data.data.len() as u64 - 1;
            trace!("new max: {}", self.max);
        }
        if (self.next == 0) && (self.min > 0) && (self.max - self.min >= buf_len) {
            trace!("max overrun while self.next == 0. (max {} min {} buf_len {}). Setting min = {}", self.max, self.min, buf_len, self.max - buf_len + 1);
            self.min = self.max - buf_len + 1
        }
        if (self.next == 0) && ( (self.min == 0) || (self.min > data.pos) ) {
            trace!("Setting min to {}", data.pos);
            self.min = data.pos;
        }
        if (self.next != 0) && (self.max - self.next >= buf_len) {
            // TODO: Set self.next to which value here?
            self.next = self.max - buf_len + 1;
            trace!("next overrun to {}", self.next);
        }
        self.last_update = Instant::now();
        //trace!("set last_update to {:?}", self.last_update);
        return Ok(Some(()));
    }

}

// ========================================

pub struct AudioBuffer {
    buf_len: u32,
    spare: u64,
    idle_threshold: u32,
    rings: HashMap<u16, RingBuffer>,
}

impl AudioBuffer {
    // buf_len is needed in order to create silence and temp buffer
    pub fn new(buf_len: u32, spare: u64, idle_threshold: u32) -> AudioBuffer {
        AudioBuffer {buf_len: buf_len, spare: spare, idle_threshold: idle_threshold, rings: HashMap::new()}
    }

    pub fn store_data(&mut self, data: AudioData) -> Result<Option<()>, String> {
        // TODO: Detect buffer that do not receive any more data
        // Note: Since we automatically add new clients, each client will use a ringbuffer with the same configuration
        if ! self.rings.contains_key(&data.client_id) {
            trace!("store_data() called for non-existing client id {}", data.client_id);
            self.rings.insert(data.client_id, RingBuffer::new(self.buf_len, self.spare, self.idle_threshold));
        }
        match self.rings.get_mut(&data.client_id) {
            Some(buffer) => buffer.store_data(data),
            None => panic!("where is the client gone?!?")
        }
    }

    pub fn get_next(&mut self, len: u32) -> Option<Vec<i16>> {
        let mut vec = vec![0_i16;len as usize];

        //let mut rng = rand::thread_rng();
        //for val in &mut vec {
        //    *val = rng.gen::<i16>() / 10;
        //}

        let scaling = 1 + self.rings.len() as i16;
        let mut some = false;
        if self.rings.len() == 0 {
            trace!("no ringbuffers added yet");
            return None;
        }

        let mut min_avail = 0;
        let mut has_none = false;
        for (id, buffer) in &self.rings {
            match buffer.peek(len as u64) {
                PeekState::Ignore => continue,
                PeekState::Avail(val) => {
                    if (min_avail == 0) || (val < min_avail) {
                        min_avail = val;
                    }
                },
                PeekState::Idle => continue,
                PeekState::None => {
                    has_none = true;
                }
            }
        }
        trace!("result of availability check: has_none {} min_avail {} spare {}", has_none, min_avail, self.spare);
        if (has_none == true) && (min_avail < self.spare) {   // TODO: What: len or self.spare?
            trace!("Not all buffers have data, return None.");
            return None;
        }
        // Adding all buffers:
        for (id, buffer) in &mut self.rings {
            trace!("Calling get_next() for client {}:", id);
            let data = match buffer.get_next(len as u64) {
                Some(data) => data,
                None => {
                    trace!("No data.");
                    continue
                }
            };
            some = true;
            //trace!(" Got data for client {}", id);
            for (pos, val) in data.iter().enumerate() {
                //trace!("Adding {} to {} at pos {}", *val, vec[pos], pos);
                vec[pos] += *val / scaling;
            }
            //trace!("done");
        }
        if some {
            trace!("get_next() returns valid data");
            Some(vec)
        }
        else {
            None
        }
    }

}

// ========================================

pub struct AudioConfig<'a> {
    pub devname: &'a str,
    pub num_channels: u32,
    pub sample_rate: u32,
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
                trace!("Sample rate was changed from {} to {}", config.sample_rate, sample_rate);
            }
            hwp.set_format(Format::s16())?;
            hwp.set_access(Access::RWInterleaved)?;
            trace!("Buffersize: {:?}", hwp.get_buffer_size());
            pcm.hw_params(&hwp)?;
        };
        pcm.prepare()?;
        Ok(Player { pcm: pcm, sample_rate: sample_rate } )
    }

    pub fn get_remain(&self) -> Frames {
        let total = match self.pcm.status() {
            Ok(val) => val.get_avail_max(),
            Err(e) => {
                trace!("******ERROR: get_avail_max() returned error {}", e);
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
                trace!("******ERROR: avail_update returned error {}, call prepare", e);
                self.pcm.prepare().unwrap();
                return 0
            }
        };
        trace!("Current audio buffer status: status.get_avail_max: {}, avail_update: {}, status.get_avail: {}, total-avail {}", total, avail, self.pcm.status().unwrap().get_avail(), total-avail);
        total - avail
    }
    
    pub fn play(&self, data: Vec<i16>) {
        let io = self.pcm.io_i16().unwrap();
        trace!("calling writei of len {}", data.len());
        match io.writei(&data[..]) {
            Ok(_) => {},
            Err(err) => {
                trace!("writei caused error: {}. Writing again.", err);
                self.pcm.prepare().unwrap();
                io.writei(&data[..]).unwrap();
            }
        }
    }

    pub fn spawn_play_thread(config: &AudioConfig, read_bucket_len: u32, buffer_mutex_play: sync::Arc<sync::Mutex<AudioBuffer>>) {
        trace!("Spawning play thread");
        let player = Player::new(config).unwrap();
        let delay = 900.0 * (read_bucket_len as f32/ config.sample_rate as f32 );
        let threshold = (1.5 * read_bucket_len as f32) as Frames;
        trace!("delay {}, threshold {}", delay as u64, threshold);
        let rbl = read_bucket_len;
        let sr = config.sample_rate;
        thread::spawn(move || {
            trace!("start");
            let mut seq = 0;
            loop {
                
//*/                while player.get_remain() < threshold {
//*/                    trace!("Player needs more data, thus calling get_next() and play()");
//*/                    let data = match buffer_mutex_play.lock().unwrap().get_next(read_bucket_len) {
//*/                        Some(data) => data,
//*/                        None => {
//*/                            trace!("Player returns no data, so do not play data");
//*/                            break;
//*/                        }
//*/                    };
//*/                    player.play(data);
//*/                }
//*/                // TODO: Adapt sleep time
//*/                trace!("Ready to sleep");
                

                // Option2: Read as many buckets as available, wait an appropriate time:
//                trace!("Remain: {}", player.get_remain());
//                trace!("Reading data from ringbuffer");
//                match buffer_mutex_play.lock().unwrap().get_next(read_bucket_len) {
//                    Some(data) => {
//                        seq = seq + 1;
//                        trace!("Calling play / seq {}", seq);
//                        player.play(data);
//                        //std::thread::sleep(std::time::Duration::from_millis(50 as u64));
//                        continue;
//                    },
//                    None => {
//                        trace!("Player returns no data, so do not play data");
//                    }
//                };
//                let local_delay = if seq > 0 {
//                    900.0 * ((seq * rbl) as f32/ sr as f32 )
//                }
//                else {
//                    50.0 * ((1 * rbl) as f32/ sr as f32 )
//                };
//                trace!("Sleeping {} after seq {}", local_delay, seq);
//                std::thread::sleep(std::time::Duration::from_millis(local_delay as u64));
//                seq = 0;
//

                // Option 3: Read only one bucket
                trace!("Remain: {}", player.get_remain());
                trace!("Reading data from ringbuffer");
                match buffer_mutex_play.lock().unwrap().get_next(read_bucket_len) {
                    Some(data) => {
                        seq = seq + 1;
                        trace!("Calling play / seq {}", seq);
                        player.play(data);
                    },
                    None => {
                        trace!("Player returns no data, so do not play data");
                    }
                };
                let local_delay = if seq > 0 {
                    200.0 * ((seq * rbl) as f32/ sr as f32 )
                }
                else {
                    50.0 * ((1 * rbl) as f32/ sr as f32 )
                };
                trace!("Sleeping {} after seq {}", local_delay, seq);
                std::thread::sleep(std::time::Duration::from_millis(local_delay as u64));
                seq = 0;
}
        });
    }
    
}

// ========================================

#[allow(dead_code)]
pub struct Recorder {
    pcm: PCM,
    sample_rate: u32,  // currently not used
    client_id: u16
}

impl Recorder {

    pub fn new(config: &AudioConfig, client_id: u16) -> Result<Recorder, Box<std::error::Error>> {
        let cs = CString::new(config.devname)?;
        let pcm = PCM::open(&*cs, Direction::Capture, false)?;
        let sample_rate;
        // The following block is needed to prevent borrow compile error because of hwp
        {
            let hwp = HwParams::any(&pcm)?;
            hwp.set_channels(config.num_channels)?;
            sample_rate = hwp.set_rate_near(config.sample_rate, ValueOr::Nearest)?;
            if sample_rate != config.sample_rate {
                trace!("Sample rate was changed from {} to {}", config.sample_rate, sample_rate);
            }
            hwp.set_format(Format::s16())?;
            hwp.set_access(Access::RWInterleaved)?;
            pcm.hw_params(&hwp)?;
        }
        Ok(Recorder { pcm : pcm, sample_rate : sample_rate, client_id: client_id })
    }

    // TODO: To allow mut code in closure, we have to declare F here as FnMut, not Fn. Is this OK?
    pub fn record<F>(&self, write_bucket_len: u64, mut callback: F) -> Result<(), Box<std::error::Error>>
        where F : FnMut(AudioData) -> ()
    {
        trace!("record() start");
        self.pcm.prepare()?;
        let io = self.pcm.io_i16()?;
        let mut i: u64 = 1;
        loop {
            let mut data = AudioData{ data: vec![0_i16; write_bucket_len as usize], pos: i, client_id: self.client_id };
            io.readi(&mut data.data[..])?;
            trace!("recorder with client_id {} calls callback for data at pos {} with len {}", self.client_id, i, write_bucket_len);
            callback(data);
            i = i + write_bucket_len;
        }
    }

    pub fn spawn_record_thread(config: &AudioConfig, client_id: u16, write_bucket_len: u64, buffer_mutex_write: sync::Arc<sync::Mutex<AudioBuffer>>) {
        trace!("Spawning record thread");
        let recorder = Recorder::new(config, client_id).unwrap();
        thread::spawn(move || {
            recorder.record(write_bucket_len, |data| {
                match buffer_mutex_write.lock().unwrap().store_data(data) {
                    Ok(_) => {},
                    Err(e) => {trace!("Could not store: {:?}", e)}
                };
            }).unwrap();
        });
    }
}
