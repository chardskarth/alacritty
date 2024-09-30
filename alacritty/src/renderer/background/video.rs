/*
    Adapted from official ffmpeg examples: https://github.com/FFmpeg/FFmpeg/blob/master/doc/examples
*/

use crate::gl;
use core::slice;
use std::{
    collections::VecDeque,
    ffi::CString,
    os::{raw::c_void, unix::ffi::OsStrExt},
    path::Path,
    ptr,
    sync::Arc,
    thread::{self, sleep, JoinHandle},
    time::{Duration, Instant},
};

use parking_lot::Mutex;
use rusty_ffmpeg::ffi::{
    av_frame_alloc, av_frame_free, av_frame_unref, av_free, av_freep, av_image_alloc,
    av_packet_alloc, av_packet_free, av_packet_unref, av_q2d, av_read_frame, av_seek_frame,
    avcodec_alloc_context3, avcodec_close, avcodec_find_decoder, avcodec_flush_buffers,
    avcodec_open2, avcodec_parameters_to_context, avcodec_receive_frame, avcodec_send_packet,
    avformat_close_input, avformat_find_stream_info, avformat_open_input, sws_freeContext,
    sws_getContext, sws_scale, AVCodecContext, AVFormatContext, AVFrame, AVPacket, SwsContext,
    AVERROR, AVERROR_EOF, AVMEDIA_TYPE_VIDEO, AV_CODEC_CAP_FRAME_THREADS,
    AV_CODEC_CAP_SLICE_THREADS, AV_PIX_FMT_RGB24, AV_TIME_BASE, EAGAIN, FF_THREAD_FRAME,
    FF_THREAD_SLICE, SWS_BILINEAR,
};
use winit::window::WindowId;

use crate::{
    event::*,
    renderer,
    scheduler::{Scheduler, TimerId, Topic},
};

struct Frame {
    pub data: Vec<u8>,
    pub pts: i64,
}

impl Frame {
    fn from_buffer(buf: *const u8, len: usize, pts: i64) -> Self {
        Self { data: unsafe { slice::from_raw_parts(buf, len).to_vec() }, pts }
    }
}

struct FrameBuffer {
    frames: VecDeque<Arc<Frame>>,
    time_base: Duration,
    duration: Duration,
}

impl FrameBuffer {
    pub fn new(time_base: Duration, duration: Duration) -> Self {
        FrameBuffer { frames: VecDeque::with_capacity(60), time_base, duration }
    }

    pub fn put(&mut self, frame: Arc<Frame>) -> bool {
        if self.frames.len() > 30 {
            return false;
        }
        self.frames.push_back(frame);
        true
    }

    /// returns: (frame, wrapped_around)
    pub fn take(&mut self, time: Duration) -> (Option<Arc<Frame>>, bool) {
        let pts = time.as_micros() / self.time_base.as_micros();
        let mut i = 0;
        let mut prev_pts = i64::MIN;
        let mut wrapped_around = false;
        if time > self.duration {
            wrapped_around = true;
        }
        for frame in &self.frames {
            if prev_pts > frame.pts {
                // we have wrapped around
                wrapped_around = true;
                break;
            }
            if frame.pts as u128 >= pts {
                break;
            }
            i += 1;
            prev_pts = frame.pts;
        }

        if i >= self.frames.len() {
            return (None, wrapped_around);
        } else {
            let ret = Some(self.frames[i].clone());
            while i > 0 {
                self.frames.pop_front();
                i -= 1;
            }
            (ret, wrapped_around)
        }
    }
}

pub(super) struct Decoder {
    pub width: i32,
    pub height: i32,
    pub avg_fps: f64,
    buffer_size: i32,
    time_base: f64,
    format: *mut AVFormatContext,
    context: *mut AVCodecContext,
    origin_frame: *mut AVFrame,
    dst_data: [*mut u8; 4],
    dst_linesize: [i32; 4],
    packet: *mut AVPacket,
    sws_ctx: *mut SwsContext,
    video_stream_index: isize,
}

impl Decoder {
    pub fn new<T: AsRef<Path>>(path: T) -> Result<Self, renderer::Error> {
        let mut decoder = Decoder {
            width: 0,
            height: 0,
            avg_fps: 0.0,
            buffer_size: 0,
            time_base: 0.0,
            format: ptr::null_mut(),
            context: ptr::null_mut(),
            origin_frame: ptr::null_mut(),
            dst_data: [ptr::null_mut(); 4],
            dst_linesize: [0; 4],
            packet: ptr::null_mut(),
            sws_ctx: ptr::null_mut(),
            video_stream_index: 0,
        };

        let c_string = CString::new(path.as_ref().as_os_str().as_bytes()).unwrap();

        unsafe {
            if avformat_open_input(
                &mut decoder.format,
                c_string.as_ptr(),
                ptr::null_mut(),
                ptr::null_mut(),
            ) != 0
            {
                return Err(renderer::Error::Other("failed to avformat open".to_string()));
            }

            if avformat_find_stream_info(decoder.format, ptr::null_mut()) < 0 {
                return Err(renderer::Error::Other("failed to find stream info".to_string()));
            }

            decoder.video_stream_index = 0;
            let mut found = false;
            let format = *decoder.format;

            while decoder.video_stream_index < format.nb_streams as isize {
                let stream = **format.streams.offset(decoder.video_stream_index);
                if (*stream.codecpar).codec_type == AVMEDIA_TYPE_VIDEO {
                    found = true;
                    break;
                }
                decoder.video_stream_index += 1;
            }

            if !found {
                return Err(renderer::Error::Other("failed to find video stream".to_string()));
            }

            let stream = **format.streams.offset(decoder.video_stream_index);
            decoder.avg_fps = av_q2d(stream.avg_frame_rate);
            decoder.time_base = av_q2d(stream.time_base);

            let codec = avcodec_find_decoder((*stream.codecpar).codec_id);
            if codec.is_null() {
                return Err(renderer::Error::Other("unsupported codec".to_string()));
            }

            decoder.context = avcodec_alloc_context3(codec);
            if avcodec_parameters_to_context(decoder.context, stream.codecpar) != 0 {
                return Err(renderer::Error::Other("failed to fill codec context".to_string()));
            }

            // from https://stackoverflow.com/questions/45363566/ffmpeg-decoding-too-slow-avcodec-send-packet-avcodec-receive-frame
            (*decoder.context).thread_count = 0;
            if (*codec).capabilities & AV_CODEC_CAP_FRAME_THREADS as i32 > 0 {
                (*decoder.context).thread_type = FF_THREAD_FRAME as i32;
            } else if (*codec).capabilities & AV_CODEC_CAP_SLICE_THREADS as i32 > 0 {
                (*decoder.context).thread_type = FF_THREAD_SLICE as i32;
            } else {
                (*decoder.context).thread_count = 1; //don't use multithreading
            }

            if avcodec_open2(decoder.context, codec, ptr::null_mut()) < 0 {
                return Err(renderer::Error::Other("failed to open codec".to_string()));
            }

            decoder.height = (*decoder.context).height;
            decoder.width = (*decoder.context).width;

            decoder.origin_frame = av_frame_alloc();
            if decoder.origin_frame.is_null() {
                return Err(renderer::Error::Other("failed to allocate frame".to_string()));
            }

            decoder.buffer_size = av_image_alloc(
                decoder.dst_data.as_mut_ptr(),
                decoder.dst_linesize.as_mut_ptr(),
                decoder.width,
                decoder.height,
                AV_PIX_FMT_RGB24,
                1,
            );
            if decoder.buffer_size < 0 {
                return Err(renderer::Error::Other("failed to allocate frame buffer".to_string()));
            }

            decoder.packet = av_packet_alloc();
            if decoder.packet.is_null() {
                return Err(renderer::Error::Other("failed to allocate packet".to_string()));
            }

            decoder.sws_ctx = sws_getContext(
                (*decoder.context).width,
                (*decoder.context).height,
                (*decoder.context).pix_fmt,
                (*decoder.context).width,
                (*decoder.context).height,
                AV_PIX_FMT_RGB24,
                SWS_BILINEAR as i32,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
            );
        }
        Ok(decoder)
    }

    pub fn is_image(&self) -> bool {
        unsafe { (*self.format).duration <= 0 }
    }

    pub fn read_frame(&mut self) -> Result<(*mut u8, i64), renderer::Error> {
        let mut pts = 0;
        let mut has_frame = false;
        unsafe {
            while av_read_frame(self.format, self.packet) >= 0 {
                if (*self.packet).stream_index as isize == self.video_stream_index {
                    if avcodec_send_packet(self.context, self.packet) < 0 {
                        return Err(renderer::Error::Other(
                            "failed to send packet for decoding".to_string(),
                        ));
                    }

                    let ret = avcodec_receive_frame(self.context, self.origin_frame);

                    if ret == AVERROR(EAGAIN) {
                        av_packet_unref(self.packet);
                        continue;
                    } else if ret == AVERROR_EOF {
                        av_seek_frame(self.format, self.video_stream_index as i32, 0, 0);
                        avcodec_flush_buffers(self.context);
                        continue;
                    } else if ret < 0 {
                        return Err(renderer::Error::Other("failed to decode packet".to_string()));
                    }

                    pts = (*self.origin_frame).pts;
                    has_frame = true;
                    sws_scale(
                        self.sws_ctx,
                        (*self.origin_frame).data.as_ptr() as *const *const u8,
                        (*self.origin_frame).linesize.as_ptr(),
                        0,
                        (*self.context).height,
                        self.dst_data.as_mut_ptr(),
                        self.dst_linesize.as_mut_ptr(),
                    );
                    av_frame_unref(self.origin_frame);
                    break;
                } else {
                    av_packet_unref(self.packet);
                }
            }

            av_packet_unref(self.packet);
            if !has_frame {
                av_seek_frame(self.format, self.video_stream_index as i32, 0, 0);
                avcodec_flush_buffers(self.context);
                return Err(renderer::Error::Other(
                    "wrapped around".to_string(),
                ));
            }
        }
        Ok((self.dst_data[0], pts))
    }
}

unsafe impl Send for Decoder {}

impl Drop for Decoder {
    fn drop(&mut self) {
        unsafe {
            // Free the YUV frame
            av_frame_free(&mut self.origin_frame);
            av_free(self.origin_frame as *mut c_void);

            // Free RGB data
            av_freep(self.dst_data.as_mut_ptr() as *mut c_void);

            // Close the codecs
            avcodec_close(self.context);

            // Close the video file
            avformat_close_input(&mut self.format);

            // free sws context
            sws_freeContext(self.sws_ctx);

            av_packet_free(&mut self.packet);
        }
    }
}

pub struct BackgroundVideo {
    buffer: Arc<Mutex<FrameBuffer>>,
    decode_thread: JoinHandle<()>,
    instant: Option<Instant>,
    is_image: bool,
    pub width: i32,
    pub height: i32,
    pub ratio: f32,
}

impl BackgroundVideo {
    pub fn new<T: AsRef<Path>>(
        path: T,
        scheduler: &mut Scheduler,
        window_id: WindowId,
    ) -> Result<Self, renderer::Error> {
        let mut decoder = Decoder::new(path)?;
        let width = decoder.width;
        let height = decoder.height;
        let ratio = decoder.width as f32 / decoder.height as f32;
        let is_image = decoder.is_image();
        let refresh_duration = Duration::from_secs(1).div_f64(decoder.avg_fps);
        let buffer = Arc::new(Mutex::new(FrameBuffer::new(
            Duration::from_secs_f64(decoder.time_base),
            Duration::from_secs_f64(unsafe {
                (*decoder.format).duration as f64 / AV_TIME_BASE as f64
            }),
        )));
        let decoder_buffer = buffer.clone();

        let decode_thread = thread::spawn(move || loop {
            if let Ok((buf, pts)) = decoder.read_frame() {
                let frame = Arc::new(Frame::from_buffer(buf, decoder.buffer_size as usize, pts));
                loop {
                    let mut buffer = decoder_buffer.lock();
                    if buffer.put(frame.clone()) {
                        break;
                    }
                    drop(buffer);
                    sleep(Duration::from_millis(500));
                }
            }
            // read only frame and exit
            if decoder.is_image() {
                break;
            }
        });

        if !is_image {
            scheduler.schedule(
                Event::new(EventType::BackgroundVideo, window_id),
                refresh_duration,
                true,
                TimerId::new(Topic::BackgroundVideo, window_id),
            );
        }

        Ok(BackgroundVideo { buffer, decode_thread, instant: None, is_image, width, height, ratio })
    }

    pub fn update_frame(&mut self, texture: u32) {
        let inited = self.instant.is_some();

        if self.is_image && inited {
            return;
        }

        let frame = {
            let mut buf = self.buffer.lock();
            if !inited {
                let (frame, _) = buf.take(Duration::default());
                if let Some(f) = frame {
                    self.instant = Some(Instant::now());
                    f
                } else {
                    return;
                }
            } else {
                let (frame, wrapped) = buf.take(self.instant.unwrap().elapsed());
                if wrapped {
                    self.instant = None;
                }
                if let Some(f) = frame {
                    f
                } else {
                    return;
                }
            }
        };

        unsafe {
            gl::PixelStorei(gl::UNPACK_ROW_LENGTH, self.width);
            gl::BindTexture(gl::TEXTURE_2D, texture);
            gl::TexImage2D(
                gl::TEXTURE_2D,
                0,
                gl::RGB as i32,
                self.width,
                self.height,
                0,
                gl::RGB,
                gl::UNSIGNED_BYTE,
                frame.data.as_ptr() as *const c_void,
            );
            gl::BindTexture(gl::TEXTURE_2D, 0);
            gl::PixelStorei(gl::UNPACK_ROW_LENGTH, 0);
        }
    }
}


