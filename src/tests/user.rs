use std::sync::atomic::Ordering;

use conduit::{Handler, Method};

use cargo_registry::Model;
use cargo_registry::token::ApiToken;
use cargo_registry::krate::EncodableCrate;
use cargo_registry::user::{User, NewUser, EncodableUser};
use cargo_registry::version::EncodableVersion;

use diesel::prelude::*;

#[derive(RustcDecodable)]
struct AuthResponse {
    url: String,
    state: String,
}
#[derive(RustcDecodable)]
pub struct UserShowResponse {
    pub user: EncodableUser,
}

#[test]
fn auth_gives_a_token() {
    let (_b, app, middle) = ::app();
    let mut req = ::req(app, Method::Get, "/authorize_url");
    let mut response = ok_resp!(middle.call(&mut req));
    let json: AuthResponse = ::json(&mut response);
    assert!(json.url.contains(&json.state));
}

#[test]
fn access_token_needs_data() {
    let (_b, app, middle) = ::app();
    let mut req = ::req(app, Method::Get, "/authorize");
    let mut response = ok_resp!(middle.call(&mut req));
    let json: ::Bad = ::json(&mut response);
    assert!(json.errors[0].detail.contains("invalid state"));
}

#[test]
fn user_insert() {
    let (_b, app, _middle) = ::app();
    let conn = t!(app.database.get());
    let tx = t!(conn.transaction());

    let user = t!(User::find_or_insert(&tx, 1, "foo", None, None, None, "bar"));
    assert_eq!(t!(User::find(&tx, user.id)), user);

    assert_eq!(
        t!(User::find_or_insert(&tx, 1, "foo", None, None, None, "bar")),
        user
    );
    let user2 = t!(User::find_or_insert(&tx, 1, "foo", None, None, None, "baz"));
    assert!(user != user2);
    assert_eq!(user.id, user2.id);
    assert_eq!(user2.gh_access_token, "baz");

    let user3 = t!(User::find_or_insert(&tx, 1, "bar", None, None, None, "baz"));
    assert!(user != user3);
    assert_eq!(user.id, user3.id);
    assert_eq!(user3.gh_login, "bar");
}

#[test]
fn me() {
    let (_b, app, middle) = ::app();
    let mut req = ::req(app, Method::Get, "/me");
    let response = t_resp!(middle.call(&mut req));
    assert_eq!(response.status.0, 403);

    let user = ::mock_user(&mut req, ::user("foo"));

    let mut response = ok_resp!(middle.call(&mut req));
    let json: UserShowResponse = ::json(&mut response);

    assert_eq!(json.user.email, user.email);
}

#[test]
fn show() {
    let (_b, app, middle) = ::app();
    {
        let conn = t!(app.diesel_database.get());

        t!(NewUser::new(1, "foo", Some("foo@bar.com"), None, None, "bar").create_or_update(&conn));
        t!(NewUser::new(2, "bar", Some("bar@baz.com"), None, None, "bar").create_or_update(&conn));
    }

    let mut req = ::req(app.clone(), Method::Get, "/api/v1/users/foo");
    let mut response = ok_resp!(middle.call(&mut req));
    let json: UserShowResponse = ::json(&mut response);
    assert_eq!(Some("foo@bar.com".into()), json.user.email);
    assert_eq!("foo", json.user.login);

    let mut response = ok_resp!(middle.call(req.with_path("/api/v1/users/bar")));
    let json: UserShowResponse = ::json(&mut response);
    assert_eq!(Some("bar@baz.com".into()), json.user.email);
    assert_eq!("bar", json.user.login);
    assert_eq!(Some("https://github.com/bar".into()), json.user.url);
}

#[test]
fn crates_by_user_id() {
    let (_b, app, middle) = ::app();
    let u;
    {
        let conn = app.diesel_database.get().unwrap();
        u = ::new_user("foo").create_or_update(&conn).unwrap();
        ::CrateBuilder::new("foo_my_packages", u.id).expect_build(&conn);
    }

    let mut req = ::req(app, Method::Get, "/api/v1/crates");
    req.with_query(&format!("user_id={}", u.id));
    let mut response = ok_resp!(middle.call(&mut req));

    #[derive(RustcDecodable)]
    struct Response {
        crates: Vec<EncodableCrate>,
    }
    let response: Response = ::json(&mut response);
    assert_eq!(response.crates.len(), 1);
}

#[test]
fn following() {
    #[derive(RustcDecodable)]
    struct R {
        versions: Vec<EncodableVersion>,
        meta: Meta,
    }
    #[derive(RustcDecodable)]
    struct Meta {
        more: bool,
    }

    let (_b, app, middle) = ::app();
    let mut req = ::req(app.clone(), Method::Get, "/");
    {
        let conn = app.diesel_database.get().unwrap();
        let user = ::new_user("foo").create_or_update(&conn).unwrap();
        ::sign_in_as(&mut req, &user);

        ::CrateBuilder::new("foo_fighters", user.id)
            .version(::VersionBuilder::new("1.0.0"))
            .expect_build(&conn);

        ::CrateBuilder::new("bar_fighters", user.id)
            .version(::VersionBuilder::new("1.0.0"))
            .expect_build(&conn);
    }

    let mut response = ok_resp!(middle.call(
        req.with_path("/me/updates").with_method(Method::Get),
    ));
    let r = ::json::<R>(&mut response);
    assert_eq!(r.versions.len(), 0);
    assert_eq!(r.meta.more, false);

    ok_resp!(
        middle.call(
            req.with_path("/api/v1/crates/foo_fighters/follow")
                .with_method(Method::Put),
        )
    );
    ok_resp!(
        middle.call(
            req.with_path("/api/v1/crates/bar_fighters/follow")
                .with_method(Method::Put),
        )
    );

    let mut response = ok_resp!(middle.call(
        req.with_path("/me/updates").with_method(Method::Get),
    ));
    let r = ::json::<R>(&mut response);
    assert_eq!(r.versions.len(), 2);
    assert_eq!(r.meta.more, false);

    let mut response = ok_resp!(
        middle.call(
            req.with_path("/me/updates")
                .with_method(Method::Get)
                .with_query("per_page=1"),
        )
    );
    let r = ::json::<R>(&mut response);
    assert_eq!(r.versions.len(), 1);
    assert_eq!(r.meta.more, true);

    ok_resp!(
        middle.call(
            req.with_path("/api/v1/crates/bar_fighters/follow")
                .with_method(Method::Delete),
        )
    );
    let mut response = ok_resp!(
        middle.call(
            req.with_path("/me/updates")
                .with_method(Method::Get)
                .with_query("page=2&per_page=1"),
        )
    );
    let r = ::json::<R>(&mut response);
    assert_eq!(r.versions.len(), 0);
    assert_eq!(r.meta.more, false);

    bad_resp!(middle.call(req.with_query("page=0")));
}

#[test]
fn user_total_downloads() {
    use diesel::update;

    let (_b, app, middle) = ::app();
    let u;
    {
        let conn = app.diesel_database.get().unwrap();

        u = ::new_user("foo").create_or_update(&conn).unwrap();

        let mut krate = ::CrateBuilder::new("foo_krate1", u.id).expect_build(&conn);
        krate.downloads = 10;
        update(&krate).set(&krate).execute(&*conn).unwrap();

        let mut krate2 = ::CrateBuilder::new("foo_krate2", u.id).expect_build(&conn);
        krate2.downloads = 20;
        update(&krate2).set(&krate2).execute(&*conn).unwrap();

        let another_user = ::new_user("bar").create_or_update(&conn).unwrap();

        let mut another_krate = ::CrateBuilder::new("bar_krate1", another_user.id)
            .expect_build(&conn);
        another_krate.downloads = 2;
        update(&another_krate)
            .set(&another_krate)
            .execute(&*conn)
            .unwrap();
    }

    let mut req = ::req(app, Method::Get, &format!("/api/v1/users/{}/stats", u.id));
    let mut response = ok_resp!(middle.call(&mut req));

    #[derive(RustcDecodable)]
    struct Response {
        total_downloads: i64,
    }
    let response: Response = ::json(&mut response);
    assert_eq!(response.total_downloads, 30);
    assert!(response.total_downloads != 32);
}

#[test]
fn updating_existing_user_doesnt_change_api_token() {
    let (_b, app, _middle) = ::app();
    let conn = t!(app.diesel_database.get());

    let gh_user_id = ::NEXT_ID.fetch_add(1, Ordering::SeqCst) as i32;

    let original_user =
        t!(NewUser::new(gh_user_id, "foo", None, None, None, "foo_token").create_or_update(&conn));
    let token = t!(ApiToken::insert(&conn, original_user.id, "foo"));

    t!(NewUser::new(gh_user_id, "bar", None, None, None, "bar_token").create_or_update(&conn));
    let user = t!(User::find_by_api_token(&conn, &token.token));

    assert_eq!("bar", user.gh_login);
    assert_eq!("bar_token", user.gh_access_token);
}
