//! Preview addresses and labels: which environment an address maps to, and
//! reading it back.

use demi_preview_rewrite::address::{
    Environment, Role, label, logical_url, map_module_specifier, map_url, map_written_url, site_of, with_parameter,
};

use demi_preview_rewrite::address::Context;

use super::{BOOT, HOST, NAMESPACE, preview_origin, top_document};

#[test]
fn one_environment_always_gets_the_same_label_and_another_environment_another() {
    let environment = Environment { origin: "http://localhost:5173".into(), top: "http://localhost".into(), cross: false };
    let first = label(NAMESPACE, HOST, &environment);
    // What the browser stored under a label is there the next time only if
    // the label of an environment never changes, in any release.
    assert_eq!(first, "selbnt2qp6d94in3");
    assert_eq!(first.len(), 16);
    let others = [
        label("other-ns", HOST, &environment),
        label(NAMESPACE, "host-2", &environment),
        label(NAMESPACE, HOST, &Environment { origin: "http://localhost:3000".into(), ..environment.clone() }),
        label(NAMESPACE, HOST, &Environment { top: "https://example.com".into(), ..environment.clone() }),
        label(NAMESPACE, HOST, &Environment { cross: true, ..environment.clone() }),
    ];
    for other in others {
        assert_ne!(other, first);
    }
}

#[test]
fn an_address_written_as_a_number_is_its_own_site() {
    // The engine, with the public suffix list, and the runtime, without it,
    // must agree, or a development page at an address gets two labels.
    assert_eq!(site_of("http://127.0.0.1:5173"), "http://127.0.0.1");
    assert_eq!(site_of("https://192.168.1.20"), "https://192.168.1.20");
    assert_eq!(site_of("http://[::1]:3000"), "http://::1");
    assert_eq!(site_of("https://www.example.co.uk"), "https://example.co.uk");
}

#[test]
fn absolute_addresses_map_to_the_label_of_the_environment_they_load_in() {
    let context = top_document("http://localhost:5173");
    let own = preview_origin("http://localhost:5173", "http://localhost", false);
    // Same origin: the document's own label, the path as written.
    assert_eq!(
        map_url("http://localhost:5173/api/items?page=2#top", None, Role::Resource, &context).unwrap(),
        format!("{own}/api/items?page=2#top")
    );
    // A subresource of another site loads under this top, cross-site.
    let cdn = preview_origin("https://cdn.example.com", "http://localhost", true);
    assert_eq!(
        map_url("https://cdn.example.com/logo.png", None, Role::Resource, &context).unwrap(),
        format!("{cdn}/logo.png")
    );
    // A same-site subresource on another port is not cross-site.
    let api = preview_origin("http://localhost:8080", "http://localhost", false);
    assert_eq!(map_url("http://localhost:8080/v1", None, Role::Resource, &context).unwrap(), format!("{api}/v1"));
    // The top document navigating itself to another site becomes that site's
    // top, through its boot page, since its forwarder may not be installed.
    let other = preview_origin("https://other.example", "https://other.example", false);
    assert_eq!(
        map_url("https://other.example/login?next=/", None, Role::Navigation, &context).unwrap(),
        format!("{other}{BOOT}#to=/login?next=/")
    );
    // Other schemes stay as they are.
    assert_eq!(map_url("data:text/plain,hi", None, Role::Resource, &context).unwrap(), "data:text/plain,hi");
}

#[test]
fn a_preview_domain_served_over_http_maps_to_http_with_its_port() {
    // A development deployment's domain (`DEMI_PREVIEW_DOMAIN`).
    let context = Context {
        scheme: "http".into(),
        domain: "demi-preview.localhost:5174".into(),
        ..top_document("http://localhost:5173")
    };
    let environment = Environment { origin: "http://localhost:5173".into(), top: "http://localhost".into(), cross: false };
    let own = format!("http://{NAMESPACE}--{}.demi-preview.localhost:5174", label(NAMESPACE, HOST, &environment));
    assert_eq!(map_url("http://localhost:5173/a", None, Role::Resource, &context).unwrap(), format!("{own}/a"));
    assert_eq!(logical_url(&format!("{own}/a"), &context).as_deref(), Some("http://localhost:5173/a"));
    // The same host over the other scheme is no preview address, either way.
    let https = own.replacen("http://", "https://", 1);
    assert_eq!(logical_url(&format!("{https}/a"), &context).as_deref(), Some(format!("{https}/a").as_str()));
    let public = top_document("http://localhost:5173");
    let plain = preview_origin("http://localhost:5173", "http://localhost", false).replacen("https://", "http://", 1);
    assert!(!public.is_preview_url(&format!("{plain}/a")));
    assert!(public.is_preview_url(&format!("{}/a", preview_origin("http://localhost:5173", "http://localhost", false))));
}

#[test]
fn relative_addresses_written_in_markup_stay_as_written() {
    let context = top_document("http://localhost:5173");
    let base = "http://localhost:5173/docs/";
    assert_eq!(map_written_url("guide.html", base, Role::Resource, &context), "guide.html");
    assert_eq!(map_written_url("/assets/app.css", base, Role::Resource, &context), "/assets/app.css");
    // A scheme-relative address takes the document's scheme, and maps.
    let cdn = preview_origin("http://cdn.example.com", "http://localhost", true);
    assert_eq!(
        map_written_url("//cdn.example.com/lib.js", base, Role::Resource, &context),
        format!("{cdn}/lib.js")
    );
}

#[test]
fn a_mapped_address_reads_back_as_the_page_wrote_it_and_an_unknown_label_does_not() {
    let context = top_document("http://localhost:5173");
    let written = "https://cdn.example.com/a.js?v=1";
    let mapped = map_url(written, None, Role::Resource, &context).unwrap();
    assert_eq!(logical_url(&mapped, &context).unwrap(), written);
    // The engine's parameters are the preview's, never part of the page's address.
    let checked = with_parameter(&mapped, "integrity", "sha384-abc");
    assert_ne!(checked, mapped);
    assert_eq!(logical_url(&checked, &context).unwrap(), written);
    // A label is a digest: another context that never met it cannot reverse it.
    let stranger = top_document("http://localhost:3000");
    assert_eq!(logical_url(&mapped, &stranger), None);
    stranger.learn(context.labels());
    assert_eq!(logical_url(&mapped, &stranger).unwrap(), written);
}

#[test]
fn module_specifiers_map_when_absolute_or_resolvable_and_bare_names_stay() {
    let context = top_document("http://localhost:5173");
    let own = preview_origin("http://localhost:5173", "http://localhost", false);
    let base = Some("http://localhost:5173/src/main.js");
    assert_eq!(map_module_specifier("vue", base, &context), "vue");
    assert_eq!(map_module_specifier("./app.js", base, &context), format!("{own}/src/app.js"));
    assert_eq!(map_module_specifier("./app.js", None, &context), "./app.js");
    let esm = preview_origin("https://esm.sh", "http://localhost", true);
    assert_eq!(map_module_specifier("https://esm.sh/react", None, &context), format!("{esm}/react"));
}
