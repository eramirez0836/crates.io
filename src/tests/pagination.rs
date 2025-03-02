use crate::builders::CrateBuilder;
use crate::util::{RequestHelper, TestApp};
use http::status::StatusCode;
use ipnetwork::IpNetwork;
use serde_json::json;

#[test]
fn pagination_blocks_ip_from_cidr_block_list() {
    let (app, anon, user) = TestApp::init()
        .with_config(|config| {
            config.max_allowed_page_offset = 1;
            config.page_offset_cidr_blocklist = vec!["127.0.0.1/24".parse::<IpNetwork>().unwrap()];
        })
        .with_user();
    let user = user.as_model();

    app.db(|conn| {
        CrateBuilder::new("pagination_links_1", user.id).expect_build(conn);
        CrateBuilder::new("pagination_links_2", user.id).expect_build(conn);
        CrateBuilder::new("pagination_links_3", user.id).expect_build(conn);
    });

    let response = anon.get_with_query::<()>("/api/v1/crates", "page=2&per_page=1");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.into_json(),
        json!({ "errors": [{ "detail": "requested page offset is too large" }] })
    );
}
