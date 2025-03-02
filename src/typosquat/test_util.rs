use std::collections::BTreeMap;

use diesel::{prelude::*, PgConnection};

use crate::{
    models::{
        Crate, CrateOwner, NewCrate, NewTeam, NewUser, NewVersion, Owner, OwnerKind, User, Version,
    },
    schema::{crate_owners, crates},
    Emails,
};

pub struct Faker {
    conn: PgConnection,
    emails: Emails,
    id: i32,
}

impl Faker {
    pub fn new(conn: PgConnection) -> Self {
        Self {
            conn,
            emails: Emails::new_in_memory(),
            id: Default::default(),
        }
    }

    pub fn borrow_conn(&mut self) -> &mut PgConnection {
        &mut self.conn
    }

    pub fn into_conn(self) -> PgConnection {
        self.conn
    }

    pub fn add_crate_to_team(
        &mut self,
        user: &User,
        krate: &Crate,
        team: &Owner,
    ) -> anyhow::Result<()> {
        // We have to do a bunch of this by hand, since normally adding a team owner triggers
        // various checks.
        diesel::insert_into(crate_owners::table)
            .values(&CrateOwner {
                crate_id: krate.id,
                owner_id: team.id(),
                created_by: user.id,
                owner_kind: OwnerKind::Team,
                email_notifications: true,
            })
            .execute(&mut self.conn)?;

        Ok(())
    }

    pub fn crate_and_version(
        &mut self,
        name: &str,
        description: &str,
        user: &User,
        downloads: i32,
    ) -> anyhow::Result<(Crate, Version)> {
        let krate = NewCrate {
            name,
            description: Some(description),
            ..Default::default()
        }
        .create(&mut self.conn, user.id)?;

        diesel::update(crates::table)
            .filter(crates::id.eq(krate.id))
            .set(crates::downloads.eq(downloads))
            .execute(&mut self.conn)?;

        let version = NewVersion::new(
            krate.id,
            &semver::Version::parse("1.0.0")?,
            &BTreeMap::new(),
            None,
            0,
            user.id,
            "0000000000000000000000000000000000000000000000000000000000000000".to_string(),
            None,
            None,
        )
        .unwrap()
        .save(&mut self.conn, "someone@example.com")
        .unwrap();

        Ok((krate, version))
    }

    pub fn team(&mut self, org: &str, team: &str) -> anyhow::Result<Owner> {
        Ok(Owner::Team(
            NewTeam::new(
                &format!("github:{org}:{team}"),
                self.next_id(),
                self.next_id(),
                Some(team.to_string()),
                None,
            )
            .create_or_update(&mut self.conn)?,
        ))
    }

    pub fn user(&mut self, login: &str) -> anyhow::Result<User> {
        Ok(
            NewUser::new(self.next_id(), login, None, None, "token").create_or_update(
                None,
                &self.emails,
                &mut self.conn,
            )?,
        )
    }

    fn next_id(&mut self) -> i32 {
        self.id += 1;
        self.id
    }
}
