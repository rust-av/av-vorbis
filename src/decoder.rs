use codec::decoder::*;
use codec::error::*;
use data::audiosample::formats::S16;
use data::audiosample::ChannelMap;
use data::frame::*;
use data::packet::Packet;
use lewton::audio::get_decoded_sample_count;
use lewton::audio::{read_audio_packet, PreviousWindowRight};
use lewton::header::read_header_setup;
use lewton::header::HeaderSet;
use lewton::header::{read_header_comment, read_header_ident};
use std::collections::VecDeque;
use std::sync::Arc;

pub struct Des {
    descr: Descr,
}

pub struct Dec {
    extradata: Option<Vec<u8>>,
    headers: Option<HeaderSet>,
    pwr: PreviousWindowRight,
    pending: VecDeque<ArcFrame>,
    info: AudioInfo,
}

impl Dec {
    fn new() -> Self {
        Dec {
            extradata: None,
            headers: None,
            pwr: PreviousWindowRight::new(),
            pending: VecDeque::with_capacity(1),
            info: AudioInfo {
                samples: 0,
                sample_rate: 48000,
                map: ChannelMap::new(),
                format: Arc::new(S16),
                block_len: None,
            },
        }
    }
}

impl Descriptor for Des {
    type OutputDecoder = Dec;

    fn create(&self) -> Self::OutputDecoder {
        Dec::new()
    }

    fn describe(&self) -> &Descr {
        &self.descr
    }
}

impl Decoder for Dec {
    fn set_extradata(&mut self, extra: &[u8]) {
        self.extradata = Some(Vec::from(extra));
    }
    fn send_packet(&mut self, pkt: &Packet) -> Result<()> {
        let headers = self.headers.as_ref().unwrap();
        let mut info = self.info.clone();
        let samples_per_channel =
            get_decoded_sample_count(&headers.0, &headers.2, pkt.data.as_slice())
                .map_err(|_e| Error::InvalidData)?;
        let channel_count = headers.0.audio_channels as usize;
        info.samples = samples_per_channel * channel_count;

        let ret = read_audio_packet(&headers.0, &headers.2, pkt.data.as_slice(), &mut self.pwr);

        if let Ok(samples) = ret {
            let mut f = Frame::new_default_frame(info, Some(pkt.t.clone()));
            {
                let buf: &mut [i16] = f.buf.as_mut_slice(0).unwrap();
                let sample_count = samples[0].len();
                for i in 0..sample_count {
                    for (cn, chan) in samples.iter().enumerate() {
                        buf[i * channel_count + cn] = chan[i];
                    }
                }
            }
            self.pending.push_back(Arc::new(f));
            Ok(())
        } else {
            Err(Error::InvalidData)
        }
    }
    fn receive_frame(&mut self) -> Result<ArcFrame> {
        self.pending.pop_front().ok_or(Error::MoreDataNeeded)
    }
    fn configure(&mut self) -> Result<()> {
        let mut extradata = if let Some(ref extradata) = self.extradata {
            extradata.as_slice()
        } else {
            return Err(Error::ConfigurationIncomplete);
        };
        // We must start with a 2 as per matroska encapsulation spec
        if extradata.is_empty() || extradata[0] != 2 {
            return Err(Error::InvalidData);
        }
        extradata = &extradata[1..];
        let ident_len = read_xiph_lacing(&mut extradata)? as usize;
        let comment_len = read_xiph_lacing(&mut extradata)? as usize;

        let ident = read_header_ident(&extradata[0..ident_len]).map_err(|_e| Error::InvalidData)?;
        extradata = &extradata[ident_len..];
        let comment =
            read_header_comment(&extradata[0..comment_len]).map_err(|_e| Error::InvalidData)?;
        extradata = &extradata[comment_len..];
        let setup = read_header_setup(
            extradata,
            ident.audio_channels,
            (ident.blocksize_0, ident.blocksize_1),
        )
        .map_err(|_e| Error::InvalidData)?;

        self.info.sample_rate = ident.audio_sample_rate as usize;
        self.info.map = ChannelMap::default_map(ident.audio_channels as usize);

        let headers = (ident, comment, setup);
        self.headers = Some(headers);
        Ok(())
    }

    fn flush(&mut self) -> Result<()> {
        self.pwr = PreviousWindowRight::new();
        Ok(())
    }
}

fn read_xiph_lacing(arr: &mut &[u8]) -> Result<u64> {
    let mut r = 0;
    loop {
        if arr.is_empty() {
            return Err(Error::InvalidData);
        }
        let v = arr[0] as u64;
        *arr = &arr[1..];
        r += v;
        if v < 255 {
            return Ok(r);
        }
    }
}

pub const VORBIS_DESCR: &Des = &Des {
    descr: Descr {
        codec: "vorbis",
        name: "lewton",
        desc: "lewton vorbis decoder",
        mime: "audio/VORBIS",
    },
};
