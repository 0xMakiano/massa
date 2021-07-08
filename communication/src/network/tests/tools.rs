use super::mock_establisher::MockEstablisherInterface;
use crate::common::NodeId;
use crate::network::{
    ConnectionId, NetworkCommandSender, NetworkConfig, NetworkEvent, NetworkEventReceiver, PeerInfo,
};
use models::SerializationContext;
use rand::{rngs::StdRng, RngCore, SeedableRng};
use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::Path,
    time::Duration,
};
use tempfile::NamedTempFile;
use time::UTime;
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    time::timeout,
};

pub const BASE_NETWORK_CONTROLLER_IP: IpAddr = IpAddr::V4(Ipv4Addr::new(169, 202, 0, 10));

// generate a named temporary JSON peers file
pub fn generate_peers_file(peer_vec: &Vec<PeerInfo>) -> NamedTempFile {
    use std::io::prelude::*;
    let peers_file_named = NamedTempFile::new().expect("cannot create temp file");
    serde_json::to_writer_pretty(peers_file_named.as_file(), &peer_vec)
        .expect("unable to write peers file");
    peers_file_named
        .as_file()
        .seek(std::io::SeekFrom::Start(0))
        .expect("could not seek file");
    peers_file_named
}

// create a NetworkConfig with typical values
pub fn create_network_config(
    network_controller_port: u16,
    peers_file_path: &Path,
) -> (NetworkConfig, SerializationContext) {
    (
        NetworkConfig {
            bind: format!("0.0.0.0:{}", network_controller_port)
                .parse()
                .unwrap(),
            routable_ip: Some(BASE_NETWORK_CONTROLLER_IP),
            protocol_port: network_controller_port,
            connect_timeout: UTime::from(3000),
            peers_file: peers_file_path.to_path_buf(),
            target_out_connections: 10,
            wakeup_interval: UTime::from(3000),
            max_in_connections: 100,
            max_in_connections_per_ip: 100,
            max_out_connnection_attempts: 100,
            max_idle_peers: 100,
            max_banned_peers: 100,
            max_advertise_length: 10,
            peers_file_dump_interval: UTime::from(30000),
            max_message_size: 3 * 1024 * 1024,
            message_timeout: UTime::from(5000u64),
            ask_peer_list_interval: UTime::from(50000u64),
        },
        SerializationContext {
            max_block_size: 1024 * 1024,
            max_block_operations: 1024,
            parent_count: 2,
            max_peer_list_length: 128,
            max_message_size: 3 * 1024 * 1024,
        },
    )
}

// ensures that the reader and writer can communicate
pub async fn expect_reader_writer_communication<ReaderT, WriterT>(
    reader: &mut ReaderT,
    writer: &mut WriterT,
    timeout_ms: u64,
) where
    ReaderT: AsyncRead + Send + Sync + Unpin + std::fmt::Debug,
    WriterT: AsyncWrite + Send + Sync + Unpin + std::fmt::Debug,
{
    let mut random_bytes_send = vec![0u8; 32];
    StdRng::from_entropy().fill_bytes(&mut random_bytes_send);
    let mut random_bytes_recv = vec![0u8; 32];
    timeout(Duration::from_millis(timeout_ms), async move {
        writer
            .write_all(&random_bytes_send)
            .await
            .expect("failed to send data on writer");
        reader
            .read_exact(&mut random_bytes_recv)
            .await
            .expect("failed to read data on reader");
        if random_bytes_send != random_bytes_recv {
            panic!("unexpected bytes received");
        }
    })
    .await
    .expect("communication test timed out");
}

// establish a full alive connection to the controller
// note: panics if any other NetworkEvent is received before NewConnection
pub async fn full_connection_to_controller(
    network_command_sender: &mut NetworkCommandSender,
    network_event_receiver: &mut NetworkEventReceiver,
    mock_interface: &mut MockEstablisherInterface,
    mock_addr: SocketAddr,
    connect_timeout_ms: u64,
    event_timeout_ms: u64,
    rw_timeout_ms: u64,
) -> NodeId {
    // establish connection towards controller
    let (mut mock_reader, mut mock_writer) = timeout(
        Duration::from_millis(connect_timeout_ms),
        mock_interface.connect_to_controller(&mock_addr),
    )
    .await
    .expect("connection towards controller timed out")
    .expect("connection towards controller failed");

    // wait for a NetworkEvent::NewConnection event
    let node_id = match timeout(
        Duration::from_millis(event_timeout_ms),
        network_event_receiver.wait_event(),
    )
    .await
    .expect("Failed waiting for a network event")
    {
        Ok(NetworkEvent::NewConnection((node_id))) => node_id,
        event @ Ok(_) => panic!("unexpected event sent by controller: {:?}", event),
        Err(_) => panic!("timeout while waiting for NewConnection event"),
    };

    node_id
}

// establish a full alive connection from the network controller
// note: fails if the controller attempts a connection to another IP first
// note: panics if any other NetworkEvent is received before NewConnection
pub async fn full_connection_from_controller(
    network_command_sender: &mut NetworkCommandSender,
    network_event_receiver: &mut NetworkEventReceiver,
    mock_interface: &mut MockEstablisherInterface,
    peer_addr: SocketAddr,
    connect_timeout_ms: u64,
    event_timeout_ms: u64,
    rw_timeout_ms: u64,
) -> NodeId {
    // wait for the incoming connection attempt, check address and accept
    let (mut mock_reader, mut mock_writer, ctl_addr, resp_tx) = timeout(
        Duration::from_millis(connect_timeout_ms),
        mock_interface.wait_connection_attempt_from_controller(),
    )
    .await
    .expect("timed out while waiting for connection from controller")
    .expect("failed getting connection from controller");
    assert_eq!(ctl_addr, peer_addr, "unexpected controller IP");
    resp_tx.send(true).expect("resp_tx failed");

    // wait for a NetworkEvent::NewConnection event
    let node_id = match timeout(
        Duration::from_millis(event_timeout_ms),
        network_event_receiver.wait_event(),
    )
    .await
    .expect("Failed waiting for a network event")
    {
        Ok(NetworkEvent::NewConnection((node_id))) => node_id,
        event @ Ok(_) => panic!("unexpected event sent by controller: {:?}", event),
        Err(_) => panic!("timeout while waiting for NewConnection event"),
    };

    node_id
}
