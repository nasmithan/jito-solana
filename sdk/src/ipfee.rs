use {
    bincode::Options,
    crossbeam_channel::{bounded, Receiver, Sender},
    solana_sdk::signature::Signature,
    std::{
        io::Write,
        net::{IpAddr, TcpStream},
        sync::OnceLock,
    },
};

#[derive(Serialize, Deserialize)]
// timestamp in all messages is number of milliseconds since Unix Epoch
pub enum IpFeeMsg {
    // A user tx was received from the remote peer
    UserTx {
        ip: IpAddr,
        signature: Signature,
    },
    // A tx was executed and paid a fee
    Fee {
        signature: Signature,
        cu_limit: u64,
        cu_used: u64,
        fee: u64,
    },
}

// Logs an error and does nothing for all calls beyond the first
pub fn ipfee_connect(addr: &str) {
    if IP_FEE.get().is_some() {
        log::error!("ipfee_connect called more than once");
        return;
    }

    log::info!("ipfee: connecting to {addr}");

    {
        let (sender, receiver) = bounded::<IpFeeMsg>(MAX_BUFFERED_IPFEE_MESSAGES);

        match IP_FEE.set(IpFee {
            sender,
            receiver,
            default_options: bincode::DefaultOptions::new(),
        }) {
            Ok(_) => (),
            Err(_) => panic!("Failed to create IP_FEE"),
        }
    }

    let addr = addr.to_string();

    std::thread::spawn(move || {
        loop {
            // Make a connection
            let mut tcp_stream = loop {
                match TcpStream::connect(&addr) {
                    Ok(tcp_stream) => break tcp_stream,
                    Err(_) => {
                        std::thread::sleep(std::time::Duration::from_secs(1));
                    }
                }
            };
            // Clear the receiver, to eliminate old messages
            {
                let tx_ingest = IP_FEE.get().expect("ipfee channel failure (1)");
                let len = tx_ingest.receiver.len();
                for _ in 0..len {
                    tx_ingest
                        .receiver
                        .recv()
                        .expect("ipfee channel failure (2)");
                }
            }

            loop {
                // Read message from IpFee receiver and write it to tcp_stream; if error, break the loop which will
                // create a new tcp_stream
                let tx_ingest = IP_FEE.get().expect("ipfee channel failure (1)");
                let tx_ingest_msg = tx_ingest
                    .receiver
                    .recv()
                    .expect("ipfee channel failure (2)");

                match tx_ingest.default_options.serialize(&tx_ingest_msg) {
                    Ok(vec_bytes) => match tcp_stream.write_all(&vec_bytes) {
                        Ok(_) => (),
                        Err(e) => {
                            log::warn!("ipfee connection failed {e}, re-connecting");
                            break;
                        }
                    },
                    Err(e) => log::error!("Failed to serialize ipfee message because {e}"),
                }
            }
        }
    });
}

// Send IpFeeMsg to connected peer.  Message will be dropped if ipfee_connect() has not been called yet or the
// channel is full
pub fn ipfee_send(msg: IpFeeMsg) {
    if let Some(tx_ingest) = IP_FEE.get() {
        tx_ingest.sender.try_send(msg).ok();
    }
}

const MAX_BUFFERED_IPFEE_MESSAGES: usize = 10000;

struct IpFee {
    pub sender: Sender<IpFeeMsg>,

    pub receiver: Receiver<IpFeeMsg>,

    pub default_options: bincode::config::DefaultOptions,
}

static IP_FEE: OnceLock<IpFee> = OnceLock::new();
