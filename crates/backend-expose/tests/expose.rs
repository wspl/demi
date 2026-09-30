//! The URL an expose shows (`expose.md` § The expose record).

use demi_backend_expose::domain::ExposeDomain;
use demi_backend_expose::records::expose_url;
use demi_web_api::ids::ExposeId;
use url::Url;

#[test]
fn an_expose_url_takes_the_scheme_and_any_other_than_the_default_port_from_the_backend_url() {
    let id = ExposeId::try_from("k7x2maqw4p3s6tavaw2y4z6aab").unwrap();
    let local: ExposeDomain = "expose.localhost".parse().unwrap();
    let backend = Url::parse("http://localhost:3271").unwrap();
    assert_eq!(
        expose_url(&id, &local, &backend),
        format!("http://{id}.expose.localhost:3271/")
    );
    let public: ExposeDomain = "expose.demi.example".parse().unwrap();
    for backend in [
        "https://demi.example",
        "https://demi.example:443/",
        "https://demi.example/api",
    ] {
        let backend = Url::parse(backend).unwrap();
        assert_eq!(
            expose_url(&id, &public, &backend),
            format!("https://{id}.expose.demi.example/")
        );
    }
}
