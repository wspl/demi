//! Kernel command-line configuration and the Linux PID 1 boot boundary.

use std::{
    collections::BTreeMap,
    io,
    net::{IpAddr, Ipv4Addr},
};

pub struct BootConfig {
    pub backend: String,
    pub token: String,
    pub network: Option<Network>,
    pub first_boot: bool,
}
pub struct Network {
    pub address: Ipv4Addr,
    pub prefix: u8,
    pub gateway: Ipv4Addr,
    pub dns: Vec<IpAddr>,
}

impl BootConfig {
    pub fn parse(text: &str) -> io::Result<Self> {
        let mut parameters = BTreeMap::new();
        let mut word = String::new();
        let mut quoted = false;
        for character in text.chars().chain(std::iter::once(' ')) {
            match character {
                '"' => quoted = !quoted,
                character if character.is_whitespace() && !quoted => {
                    if !word.is_empty() {
                        let (key, value) = word.split_once('=').unwrap_or((&word, ""));
                        if parameters
                            .insert(key.to_owned(), value.to_owned())
                            .is_some()
                            && key.starts_with("demi.")
                        {
                            return Err(invalid("duplicate guest boot parameter"));
                        }
                        word.clear();
                    }
                }
                character => word.push(character),
            }
        }
        if quoted {
            return Err(invalid("unterminated quoted kernel parameter"));
        }
        let backend = parameters
            .remove("demi.backend")
            .filter(|value| !value.is_empty())
            .ok_or_else(|| invalid("kernel command line names no demi.backend"))?;
        crate::state::backend_url(&backend)?;
        let token = parameters
            .remove("demi.token")
            .filter(|value| !value.is_empty() && !value.chars().any(char::is_whitespace))
            .ok_or_else(|| invalid("kernel command line names no valid demi.token"))?;
        let network = match (parameters.remove("demi.ip"), parameters.remove("demi.gw")) {
            (None, None) => None,
            (Some(address), Some(gateway)) => {
                let (address, prefix) = address
                    .split_once('/')
                    .ok_or_else(|| invalid("guest IP needs a network prefix"))?;
                let address = address
                    .parse()
                    .map_err(|_| invalid("invalid guest IPv4 address"))?;
                let prefix = prefix
                    .parse::<u8>()
                    .ok()
                    .filter(|prefix| *prefix <= 32)
                    .ok_or_else(|| invalid("invalid guest network prefix"))?;
                let gateway = gateway
                    .parse()
                    .map_err(|_| invalid("invalid guest IPv4 gateway"))?;
                let dns = parameters
                    .remove("demi.dns")
                    .unwrap_or_default()
                    .split(',')
                    .filter(|value| !value.is_empty())
                    .map(|value| {
                        value
                            .parse()
                            .map_err(|_| invalid("invalid guest DNS address"))
                    })
                    .collect::<io::Result<_>>()?;
                Some(Network {
                    address,
                    prefix,
                    gateway,
                    dns,
                })
            }
            _ => return Err(invalid("guest IP and gateway must be provided together")),
        };
        let first_boot = match parameters.get("demi.firstboot").map(String::as_str) {
            None | Some("0") => false,
            Some("1") => true,
            _ => return Err(invalid("invalid first-boot marker")),
        };
        Ok(Self {
            backend,
            token,
            network,
            first_boot,
        })
    }
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::{Boot, boot, supervise};

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn boot_configuration_validates_machine_inputs_without_exposing_the_token() {
        let config = BootConfig::parse("console=ttyS0 demi.backend=\"https://example.test\" demi.token=secret demi.ip=172.16.5.2/30 demi.gw=172.16.5.1 demi.dns=1.1.1.1,::1 demi.firstboot=1").unwrap();
        assert_eq!(config.backend, "https://example.test");
        assert!(config.first_boot);
        let network = config.network.unwrap();
        assert_eq!(network.prefix, 30);
        assert_eq!(network.dns.len(), 2);
        assert!(
            BootConfig::parse(
                "demi.backend=https://example.test demi.token=a demi.ip=bad demi.gw=1.1.1.1"
            )
            .is_err()
        );
        assert!(
            BootConfig::parse("demi.backend=https://example.test demi.token=a demi.token=b")
                .is_err()
        );
        assert!(
            BootConfig::parse(
                "demi.backend=https://example.test demi.token=a demi.ip=1.1.1.1/33 demi.gw=1.1.1.2"
            )
            .is_err()
        );
        assert!(
            BootConfig::parse("demi.backend=https://example.test demi.token=\"private token\"")
                .is_err()
        );
    }
}
