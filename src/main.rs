use gst::{prelude::*, glib::ffi::GError};
use gst_sdp::sdp_message::SDPMessage;
use gst_webrtc::{WebRTCSDPType, WebRTCSessionDescription};

mod services;
use services::*;

use clap::Parser;

fn prepare_pipeline(
    video_base: String,
    audio_base: String,
) -> Result<Box<gst::Pipeline>, gst::glib::Error> {
    // Initialize gstreamer
    if let Err(e) = gst::init() {
        eprintln!("Could not initialize gstreamer");
        return Err(e);
    }

    // Build the pipeline
    let pipeline = match gst::parse_launch(&format!(
        "queue name=videoqueue ! {} queue name=audioqueue ! {}",
        video_base, audio_base
    )) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Failed to parse initial pipeline");
            return Err(e);
        }
    };

    Ok(Box::new(pipeline.dynamic_cast::<gst::Pipeline>().unwrap()))
}

fn add_webrtc(services: &Box<Services>, identifier: &String, pipeline: &Box<gst::Pipeline>) {
    println!("Adding Webrtc node");

    if let Some(_webrtc) = pipeline.by_name(String::from("webrtc").as_str()) {
        let identifier = identifier.clone();
        remove_webrtc(identifier, pipeline);
    }

    let webrtc =
        gst::ElementFactory::make("webrtcbin", Some(String::from("webrtc").as_str()))
            .expect("");
    webrtc.set_property("latency", 0 as u32);

    pipeline.add(&webrtc).unwrap();
    {
        let services = services.clone();
        let identifier = identifier.clone();
        webrtc.connect("on-ice-candidate", false, move |values| {
            println!("ICE Candidate created");
            let sdp_mline_index = values[1].get::<u32>().expect("Invalid argument");
            let candidate = values[2].get::<String>().expect("Invalid argument");

            services.send_ice_candidate(&identifier, sdp_mline_index, candidate);
            println!("ICE Candidate sent");
            None
        });
        let pipeline = pipeline.clone();
        webrtc.connect("pad-added", false, move |srcpad| {
            println!("Pad added: ");
            let srcpad = srcpad[1].get::<gst::Pad>().expect("Invalid argument");
            let current_caps = srcpad.current_caps().unwrap();
            let media_type: String = current_caps.structure(0).unwrap().get("media").unwrap();
            let encoding: String = current_caps
                .structure(0)
                .unwrap()
                .get("encoding-name")
                .unwrap();
            println!("Encoding: {}", encoding);
            let sinkpad = match media_type.as_str() {
                "video" => {
                    let video_queue = pipeline.by_name("videoqueue").unwrap();
                    video_queue.static_pad("sink").unwrap()
                },
                "audio" => {
                    let audio_queue = pipeline.by_name("audioqueue").unwrap();
                    audio_queue.static_pad("sink").unwrap()
                },
                _ => panic!("Wrong media type received !"),
            };
            srcpad.link(&sinkpad).unwrap();
            None
        });
    }

    // Start playing
    pipeline
        .set_state(gst::State::Playing)
        .expect("Unable to set the pipeline to the `Playing` state");
}

fn remove_webrtc(identifier: String, pipeline: &Box<gst::Pipeline>) {
    println!("Removing Webrtc node");

    // Pause
    pipeline
        .set_state(gst::State::Paused)
        .expect("Unable to set the pipeline to the `Playing` state");

    let webrtc = pipeline
        .by_name(String::from("webrtc").as_str())
        .unwrap();
    pipeline.remove_many(&[&webrtc]).unwrap();
}

fn handle_sdp_offer(
    services: &Box<Services>,
    identifier: &String,
    offer: gst_webrtc::WebRTCSessionDescription,
    pipeline: &Box<gst::Pipeline>,
) {
    println!("Setting remote description");
    let webrtc = pipeline
        .by_name(String::from("webrtc").as_str())
        .unwrap();
    let services = services.clone();
    let ident = identifier.clone();
    let w = webrtc.clone();
    let promise = gst::Promise::with_change_func(move |_| {
        println!("Remote description set");
        let w2 = w.clone();
        let promise = gst::Promise::with_change_func(move |reply| {
            println!("SDP Answer created");
            let reply = match reply {
                Ok(Some(reply)) => reply,
                Ok(None) => {
                    println!("No answer");
                    return;
                }
                Err(_) => {
                    println!("Error answer");
                    return;
                }
            };
            println!("{:?}", reply);
            
            match reply.value("error").map(|error| {
                error
            }) {
                Ok(o) => {
                    //println!("SDP generated Error: {:?}", o.get::<gst::message::Error>().unwrap());
                },
                Err(err) => {
                    eprintln!("SDP error getting error: {:?}", err);
                }
            };
            let answer = match reply.value("answer").map(|answer| {
                answer
            }) {
                Ok(o) => o,
                Err(err) => {
                    eprintln!("SDP answer generation error: {:?}", err);
                    return;
                }
            };
            let desc = answer
                .get::<gst_webrtc::WebRTCSessionDescription>()
                .unwrap();
            println!("SDP Type: {}", desc.type_().to_str());
            println!("SDP :\n{}", desc.sdp().as_text().unwrap());

            let w3 = w2.clone();
            let promise = gst::Promise::with_change_func(move |_| {
                services.send_sdp_offer(&ident, desc.sdp().as_text().unwrap());
                println!("SDP Answer sent\n");
                for p in w3.src_pads() {
                    println!("{:?}", p.name());
                }
            });
            w2.emit_by_name::<()>("set-local-description", &[&answer, &promise]);
        });
        w.emit_by_name::<()>("create-answer", &[&None::<gst::Structure>, &promise]);
    });
    webrtc.emit_by_name::<()>("set-remote-description", &[&offer, &promise]);
}

fn handle_ice_candidate(
    identifier: &String,
    sdp_mline_index: u32,
    candidate: &str,
    pipeline: &Box<gst::Pipeline>,
) {
    let webrtc = pipeline
        .by_name(String::from("webrtc").as_str())
        .unwrap();
    webrtc.emit_by_name::<()>("add-ice-candidate", &[&sdp_mline_index, &candidate]);
}

/// Simple program to greet a person
#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    /// Signaling server address
    #[clap(long, default_value = "localhost")]
    address: String,

    /// Signaling server port
    #[clap(long, default_value = "8000")]
    port: u32,

    /// Use SSL/TLS to contact Signaling server
    #[clap(long)]
    unsecure: bool,

    /// Gstreamer video pipeline base
    #[clap(
        long,
        default_value = "rtph264depay ! decodebin ! queue ! autovideosink sync=false"
    )]
    video_base: String,

    /// Gstreamer audio pipeline base
    #[clap(
        long,
        default_value = "rtpopusdepay ! decodebin ! queue ! autoaudiosink sync=false"
    )]
    audio_base: String,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    let pipeline = match prepare_pipeline(args.video_base, args.audio_base) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Failed to initialize pipeline:\n{:?}", e);
            return;
        }
    };

    let signaling_protocol = if args.unsecure { "http" } else { "https" };
    let uri = format!("{}://{}:{}/", signaling_protocol, args.address, args.port);

    println!("Connecting to signaling server with uri: {}", uri);
    let services = Box::new(Services::new(uri));
    {
        let pipeline = &pipeline.clone();
        let services = &services.clone();
        services
            .start_sse(
                move |sender| {
                    add_webrtc(&services, sender, &pipeline);
                },
                move |message: &IncomingMessage| {
                    println!("New incoming SDP message:");
                    if let Payload::SDP { description } = &message.payload {
                        println!("{:?}", description);
                        let sdp = match SDPMessage::parse_buffer(description.as_bytes()) {
                            Ok(r) => r,
                            Err(err) => {
                                println!("Error: Can't parse SDP Description !");
                                println!("{:?}", err);
                                return;
                            }
                        };
                        handle_sdp_offer(
                            &services,
                            &message.sender,
                            WebRTCSessionDescription::new(WebRTCSDPType::Offer, sdp),
                            &pipeline,
                        );
                    }
                },
                move |message: &IncomingMessage| {
                    println!("New incoming ICE candidate:");
                    if let Payload::ICE { index, candidate } = &message.payload {
                        println!("{:?}", candidate);
                        handle_ice_candidate(
                            &message.sender,
                            *index,
                            candidate.as_str(),
                            &pipeline,
                        );
                    };
                },
            )
            .await;
    }

    // Shutdown pipeline
    pipeline
        .set_state(gst::State::Null)
        .expect("Unable to set the pipeline to the `Null` state");
}
