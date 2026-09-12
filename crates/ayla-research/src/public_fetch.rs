//! Public research URLs are validated at every redirect, then DNS is pinned
//! for the request so an address cannot change between validation and connect.
use anyhow::{bail, Context, Result};
use std::net::IpAddr;
use std::time::Duration;
use url::Url;

pub fn public_address(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(ip) => {
            let bytes = ip.octets();
            !(ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_unspecified()
                || ip.is_multicast()
                || ip.is_broadcast()
                || bytes[0] == 0
                || bytes[0] >= 240
                || (bytes[0] == 198 && (18..=19).contains(&bytes[1]))
                || (bytes[0] == 192 && bytes[1] == 0 && bytes[2] == 0)
                || (bytes[0] == 192 && bytes[1] == 88 && bytes[2] == 99)
                || (bytes[0] == 100 && (64..=127).contains(&bytes[1]))
                || ip.is_documentation())
        }
        IpAddr::V6(ip) => {
            if let Some(mapped) = ip.to_ipv4_mapped() {
                return public_address(mapped.into());
            }
            let segments = ip.segments();
            !(ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_multicast()
                || segments[0] & 0xfe00 == 0xfc00
                || segments[0] & 0xffc0 == 0xfe80
                || segments[0] & 0xe000 != 0x2000
                || segments[0] == 0x2002
                || (segments[0] == 0x2001 && (segments[1] < 0x0200 || segments[1] == 0x0db8)))
        }
    }
}

pub async fn validate_target(url: &Url) -> Result<Vec<std::net::SocketAddr>> {
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
    {
        bail!("Research URLs must be public HTTP(S) URLs without credentials");
    }
    let host = url.host_str().context("Research URL has no host")?;
    let port = url
        .port_or_known_default()
        .context("Research URL has no port")?;
    let addresses: Vec<_> = tokio::time::timeout(
        Duration::from_secs(10),
        tokio::net::lookup_host((host.trim_matches(['[', ']']), port)),
    )
    .await??
    .collect();
    if addresses.is_empty()
        || addresses
            .iter()
            .any(|address| !public_address(address.ip()))
    {
        bail!("Research URL resolves to a non-public address");
    }
    Ok(addresses)
}

pub async fn get_public(url: &str, timeout: Duration) -> Result<reqwest::Response> {
    let mut target = Url::parse(url)?;
    for _ in 0..=5 {
        let addresses = validate_target(&target).await?;
        let mut builder = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(timeout)
            .connect_timeout(Duration::from_secs(10))
            .no_proxy()
            .user_agent("Ayla/0.1 research");
        if let Some(url::Host::Domain(host)) = target.host() {
            builder = builder.resolve_to_addrs(host, &addresses);
        }
        let response = builder.build()?.get(target.clone()).send().await?;
        if response.status().is_redirection() {
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .context("Redirect has no Location")?
                .to_str()?;
            target = target.join(location)?;
            continue;
        }
        return Ok(response);
    }
    bail!("Research URL exceeded the redirect limit")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_local_private_mapped_and_reserved_addresses() {
        for address in [
            "127.0.0.1",
            "10.1.2.3",
            "169.254.169.254",
            "100.64.1.1",
            "192.0.2.4",
            "::1",
            "::ffff:127.0.0.1",
            "fc00::1",
            "fe80::1",
            "198.18.0.1",
            "192.0.0.1",
            "::127.0.0.1",
            "64:ff9b::127.0.0.1",
            "2002:7f00:1::1",
        ] {
            assert!(!public_address(address.parse().unwrap()), "{address}");
        }
        assert!(public_address("8.8.8.8".parse().unwrap()));
        assert!(public_address("2606:4700:4700::1111".parse().unwrap()));
    }
}
