//! The backend's public address (`native-runtime.md` § Backend deployment
//! configuration): where runners, Cloud guests and expose visitors reach
//! this backend.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::{Arc, OnceLock};

use demi_runner_protocol::values::BackendUrl;
use url::Url;

/// `DEMI_BACKEND_PUBLIC_URL`, or without one (a test's backend) the
/// listener's own address. The edge sets it once the backend listens,
/// before it serves; a Cloud's boot, an expose's URL and a development
/// store's downloads name it. Cloning it shares it.
#[derive(Clone, Default)]
pub struct PublicUrl(Arc<OnceLock<BackendUrl>>);

impl PublicUrl {
    /// Sets the URL once the backend listens on `address`: `public`, or
    /// without one the listener's own address.
    pub fn listening(&self, public: Option<&Url>, address: SocketAddr) {
        let url = match public {
            Some(public) => public.as_str().parse::<BackendUrl>(),
            None => {
                // A listener on every address is reached on the loopback one.
                let ip = match address.ip() {
                    IpAddr::V4(ip) if ip.is_unspecified() => IpAddr::V4(Ipv4Addr::LOCALHOST),
                    IpAddr::V6(ip) if ip.is_unspecified() => IpAddr::V6(Ipv6Addr::LOCALHOST),
                    ip => ip,
                };
                format!("http://{}", SocketAddr::new(ip, address.port())).parse::<BackendUrl>()
            }
        };
        match url {
            // A backend listens once; a second call finds the URL set.
            Ok(url) => {
                let _ = self.0.set(url);
            }
            Err(error) => tracing::error!("the URL runners connect to is not usable: {error}"),
        }
    }

    /// The URL, once the backend listens.
    pub fn get(&self) -> Option<&BackendUrl> {
        self.0.get()
    }
}
