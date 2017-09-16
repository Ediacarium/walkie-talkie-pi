use std::collections::HashMap;
use std::thread;
use std::sync;
use std::ffi::CString;
use std;
use alsa::{Direction, ValueOr};
use alsa::pcm::{PCM, HwParams, Format, Access, Frames};


#[derive(Clone, Serialize, Deserialize)]
pub struct AudioData {
    pub client_id: u16,
    pub pos: u64,    // pos must be > 0 (because position 0 is considered to be initial (unset) value
    pub data: Vec<i16>,
}

pub struct RingBuffer {
    buf: Vec<i16>,
    next: u64,  // points to first element to read next, i.e. buf[next] was not yet read
    max: u64,   // points to the last filled element, i.e. buf[max] is set
    spare: u64, // number of samples to always keep in buffer
}

impl RingBuffer {
    pub fn new(buf_len: u32, spare: u64) -> RingBuffer {
        assert!(spare < buf_len as u64);
        RingBuffer { buf: vec![0_i16;buf_len as usize], max: 0, next: 0, spare: spare }
    }

    pub fn get_next(&mut self, target_len: u64) -> Option<Vec<i16>> {
        assert!(self.spare + target_len < self.buf.len() as u64);
        trace!("ringbuffer: next {}/{} max {}/{} spare {} len {}", self.next, self.next % self.buf.len() as u64, self.max, self.max % self.buf.len() as u64, self.spare, self.buf.len());
        let buf_len = self.buf.len() as u64;
        // TODO: Combine if clauses (added to have debug output only)
        if self.max == 0 {
            trace!("no data yet added");
            return None;
        }
        if self.max < self.spare {
            trace!("not enough data added yet");
            return None;
        }
        if self.next == 0 {
            // first call -> set self.next to some sensitive value
//            self.next = if self.max < buf_len {
//                1
//            }
//            else {
//                self.max - buf_len + 1
//            };

            self.next = if self.max > self.spare {
                self.max - self.spare
            }
            else {
                1
            };

            trace!("First call, adjusting next to {}", self.next);
        }
        if self.next > self.max - self.spare {
            trace!("not enough spare data");
            return None;
        }
        let max_avail = ((self.max - self.spare) + 1) - self.next;   // Do not change computation order to prevent underflow!
        let len = if max_avail < target_len {
            max_avail
        }
        else {
            target_len
        };
        assert!(len > 0);
        //let mut result = Vec::<i16>::with_capacity(len as usize);
        let mut result = vec![0_i16;len as usize];
        let start = (self.next % buf_len) as usize;
        let mut end_excl = ((self.next + len) % buf_len) as usize;  // buf[end_excl] is the first element not to be returned.
        if end_excl == 0 {
            end_excl = (buf_len + 1) as usize;
        }
        if start < end_excl {
            // current bucket does not exceed buf size, thus we may copy in one piece:
            trace!("get_next(): copy1 [0..{}] <- [{}..{}]", len, start, end_excl);
            &result[0..len as usize].copy_from_slice(&self.buf[start..end_excl]);  // note: start..end excludes the element at end_excl
        }
        else {
            // current bucket exceeds buf size, thus copy in two pieces:
            let len_first_part = buf_len as usize - start;
            trace!("get_next(): copy2 [0..{}] <- [{}..{}]", len_first_part, start, buf_len);
            trace!("get_next(): copy2 [{}..{}] <- [0..{}]", len_first_part, len, end_excl);
            &result[0..len_first_part].copy_from_slice(&self.buf[start..buf_len as usize]);
            &result[len_first_part..len as usize].copy_from_slice(&self.buf[0..end_excl]);
        }
        self.next = self.next + len;
        Some(result)
    }

    fn store_data(&mut self, data: AudioData) -> Result<Option<()>, String> {
        let buf_len = self.buf.len() as u64;
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
        if (self.next != 0) && (self.max - self.next >= buf_len) {
            // TODO: Set self.next to which value here?
            self.next = self.max - buf_len + 1;
            trace!("next overrun to {}", self.next);
        }
        return Ok(Some(()));
    }

}

// ========================================

pub struct AudioBuffer {
    buf_len: u32,
    spare: u64,
    rings: HashMap<u16, RingBuffer>,
}

impl AudioBuffer {
    // buf_len is needed in order to create silence and temp buffer
    pub fn new(buf_len: u32, spare: u64) -> AudioBuffer {
        AudioBuffer {buf_len: buf_len, spare: spare, rings: HashMap::new()}
    }

    pub fn store_data(&mut self, data: AudioData) -> Result<Option<()>, String> {
        // TODO: Detect buffer that do not receive any more data
        // Note: Since we automatically add new clients, each client will use a ringbuffer with the same configuration
        if ! self.rings.contains_key(&data.client_id) {
            trace!("store_data() called for non-existing client id {}", data.client_id);
            self.rings.insert(data.client_id, RingBuffer::new(self.buf_len, self.spare));
        }
        match self.rings.get_mut(&data.client_id) {
            Some(buffer) => buffer.store_data(data),
            None => panic!("where is the client gone?!?")
        }
    }

    pub fn get_next(&mut self, len: u32) -> Option<Vec<i16>> {
        let mut vec = vec![0_i16;len as usize];
        let scaling = self.rings.len() as i16;
        let mut some = false;
        if self.rings.len() == 0 {
            trace!("no ringbuffers added yet");
            return None;
        }
        // Adding all buffers:
        for (_, buffer) in &mut self.rings {
            //trace!("Calling get_next() for client {}:", id);
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
        trace!("Current audio buffer status: total: {}, avail: {}, total-avail {}", total, avail, total-avail);
        total - avail
    }
    
    pub fn play(&self, data: Vec<i16>) {
        let io = self.pcm.io_i16().unwrap();
        trace!("calling writei");
        io.writei(&data[..]).unwrap();
    }

    pub fn spawn_play_thread(config: &AudioConfig, read_bucket_len: u32, buffer_mutex_play: sync::Arc<sync::Mutex<AudioBuffer>>) {
        trace!("Spawning play thread");
        let player = Player::new(config).unwrap();
        let delay = 900.0 * (read_bucket_len as f32/ config.sample_rate as f32 );
        let threshold = (1.5 * read_bucket_len as f32) as Frames;
        trace!("delay {}, threshold {}", delay as u64, threshold);
        thread::spawn(move || {
            loop {
                while player.get_remain() < threshold {
                    trace!("Player needs more data, thus calling get_next() and play()");
                    let data = match buffer_mutex_play.lock().unwrap().get_next(read_bucket_len) {
                        Some(data) => data,
                        None => {
                            trace!("Player returns no data, so do not play data");
                            break;
                        }
                    };
                    player.play(data);
                }
                // TODO: Adapt sleep time
                trace!("Ready to sleep");
                
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
