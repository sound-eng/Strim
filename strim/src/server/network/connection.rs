/// Retrieve the current local IP address of the machine
pub fn get_local_ip() -> Option<std::net::IpAddr> {
    let udp_socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    udp_socket.connect("8.8.8.8:80").ok()?;
    udp_socket.local_addr().ok().map(|addr| addr.ip())
}

