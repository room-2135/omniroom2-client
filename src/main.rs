use gst::prelude::*;
use gst_webrtc::{WebRTCSessionDescription, WebRTCSDPType};
use gst_sdp::sdp_message::SDPMessage;

mod services;
use services::*;

fn prepare_pipeline() -> Result<Box<gst::Pipeline>, gst::glib::Error> {
    // Initialize gstreamer
    if let Err(e) = gst::init() {
        eprintln!("Could not initialize gstreamer");
        return Err(e);
    }

    // Build the pipeline
    let pipeline = match gst::parse_launch(&format!("queue name=videoqueue ! rtph264depay ! decodebin ! queue ! autovideosink sync=false")) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Failed to parse initial pipeline");
            return Err(e);
        }
    };

    Ok(Box::new(pipeline.dynamic_cast::<gst::Pipeline>().unwrap()))
}

fn add_webrtc(services: &Box<Services>, identifier: &String,  pipeline: &Box::<gst::Pipeline>) {
    println!("Adding Webrtc node");

    if let Some(_webrtc) = pipeline.by_name(format!("webrtc-{}", identifier).as_str()) {
        let identifier = identifier.clone();
        remove_webrtc(identifier, pipeline);
    } else {
        let queue = pipeline.by_name("videoqueue").unwrap();
        let webrtc = gst::ElementFactory::make("webrtcbin", Some(format!("webrtc-{}", identifier).as_str())).expect("");
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
            webrtc.connect("pad-added", false, move |srcpad| {
                println!("Pad added");
                let sinkpad = queue.static_pad("sink").unwrap();
                let srcpad = srcpad[1].get::<gst::Pad>().expect("Invalid argument");
                srcpad.link(&sinkpad).unwrap();
                None
            });
        }
    }

    // Start playing
    pipeline
        .set_state(gst::State::Playing)
        .expect("Unable to set the pipeline to the `Playing` state");
}

fn remove_webrtc(identifier: String, pipeline: &Box::<gst::Pipeline>) {
    println!("Removing Webrtc node");
    let webrtc = pipeline.by_name(format!("webrtc-{}",identifier).as_str()).unwrap();
    pipeline.remove_many(&[&webrtc]).unwrap();
}

fn handle_sdp_offer(services: &Box<Services>, identifier: &String, offer: gst_webrtc::WebRTCSessionDescription, pipeline: &Box<gst::Pipeline>) {
    println!("Setting remote description");
    let webrtc = pipeline.by_name(format!("webrtc-{}", identifier).as_str()).unwrap();
    let services = services.clone();
    let ident = identifier.clone();
    let w = webrtc.clone();
    let promise = gst::Promise::with_change_func(move |_| {
        println!("Remote description set");
        let w2 = w.clone();
        let promise = gst::Promise::with_change_func(move |reply| {
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
            let answer = match reply.value("answer").map(|answer| {
                println!("Offer created");
                answer
            }) {
                Ok(o) => o,
                Err(_) => {
                    return;
                }
            };
            let desc = answer.get::<gst_webrtc::WebRTCSessionDescription>().unwrap();
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

fn handle_ice_candidate(identifier: &String, sdp_mline_index: u32, candidate: &str, pipeline: &Box<gst::Pipeline>) {
    let webrtc = pipeline.by_name(format!("webrtc-{}", identifier).as_str()).unwrap();
    webrtc.emit_by_name::<()>("add-ice-candidate", &[&sdp_mline_index, &candidate]);
}

#[tokio::main]
async fn main() {
    let pipeline = match prepare_pipeline() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Failed to initialize pipeline:\n{:?}", e);
            return;
        }
    };

    let services = Box::new(Services::new("http://localhost:8900/".to_string()));
    {
        let pipeline = &pipeline.clone();
        let services = &services.clone();
        services.start_sse(move |message: &IncomingMessage| {
            println!("New incoming SDP message:");
            add_webrtc(services, &message.sender, pipeline);
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
                handle_sdp_offer(&services, &message.sender, WebRTCSessionDescription::new(WebRTCSDPType::Offer, sdp), &pipeline);
            }
        }, move |message: &IncomingMessage| {
            println!("New incoming ICE candidate:");
            if let Payload::ICE { index, candidate } = &message.payload {
                println!("{:?}", candidate);
                handle_ice_candidate(&message.sender, *index, candidate.as_str(), &pipeline);
            };
        }).await;
    }

    // Shutdown pipeline
    pipeline
        .set_state(gst::State::Null)
        .expect("Unable to set the pipeline to the `Null` state");
}
