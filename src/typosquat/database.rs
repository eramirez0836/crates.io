//! Types that bridge the crates.io database and typomania.

use std::{
    borrow::Borrow,
    collections::{BTreeMap, HashMap, HashSet},
};

use diesel::{connection::DefaultLoadingMode, PgConnection, QueryResult};
use typomania::{AuthorSet, Corpus, Package};

/// A corpus of the current top crates on crates.io, as determined by their download counts, along
/// with their ownership information so we can quickly check if a new crate shares owners with a
/// top crate.
pub struct TopCrates {
    pub(super) crates: HashMap<String, Crate>,
}

impl TopCrates {
    /// Retrieves the `num` top crates from the database.
    pub fn new(conn: &mut PgConnection, num: i64) -> QueryResult<Self> {
        use crate::{
            models,
            schema::{crate_owners, crates},
        };
        use diesel::prelude::*;

        // We have to build up a data structure that contains the top crates, their owners in some
        // form that is easily compared, and that can be indexed by the crate name.
        //
        // In theory, we could do this with one super ugly query that uses array_agg() and
        // implements whatever serialisation logic we want to represent owners at the database
        // level. But doing so gets rid of most of the benefits of using Diesel, and requires a
        // bunch of ugly code.
        //
        // Instead, we'll issue two queries: one to get the top crates, and then another to get all
        // their owners. This is essentially the manual version of the pattern described in the
        // Diesel relation guide's "reading data" section to zip together two result sets. We can't
        // use the actual pattern because crate_owners isn't selectable (for reasons that are
        // generally good, but annoying in this specific case).
        //
        // Once we have the results of those queries, we can glom it all together into one happy
        // data structure.

        let mut crates: BTreeMap<i32, (String, Crate)> = BTreeMap::new();
        for result in models::Crate::all()
            .order(crates::downloads.desc())
            .limit(num)
            .load_iter::<models::Crate, DefaultLoadingMode>(conn)?
        {
            let krate = result?;
            crates.insert(
                krate.id,
                (
                    krate.name,
                    Crate {
                        owners: HashSet::new(),
                    },
                ),
            );
        }

        // This query might require more low level knowledge of crate_owners than we really want
        // outside of the models module. It would probably make more sense in the long term to have
        // this live in the Owner type, but for now I want to keep the typosquatting logic as
        // self-contained as possible in case we decide not to go ahead with this in the longer
        // term.
        for result in crate_owners::table
            .filter(crate_owners::deleted.eq(false))
            .filter(crate_owners::crate_id.eq_any(crates.keys().collect::<Vec<_>>()))
            .select((
                crate_owners::crate_id,
                crate_owners::owner_id,
                crate_owners::owner_kind,
            ))
            .load_iter::<(i32, i32, i32), DefaultLoadingMode>(conn)?
        {
            let (crate_id, owner_id, owner_kind) = result?;
            crates.entry(crate_id).and_modify(|(_name, krate)| {
                krate.owners.insert(Owner::new(owner_id, owner_kind));
            });
        }

        Ok(Self {
            crates: crates.into_values().collect(),
        })
    }
}

impl Corpus for TopCrates {
    fn contains_name(&self, name: &str) -> typomania::Result<bool> {
        Ok(self.crates.contains_key(name))
    }

    fn get(&self, name: &str) -> typomania::Result<Option<&dyn Package>> {
        Ok(self.crates.get(name).map(|krate| krate as &dyn Package))
    }
}

pub struct Crate {
    owners: HashSet<Owner>,
}

impl Crate {
    /// Hydrates a crate and its owners from the database given the crate name.
    pub fn from_name(conn: &mut PgConnection, name: &str) -> QueryResult<Self> {
        use crate::models;
        use diesel::prelude::*;

        let krate = models::Crate::by_exact_name(name).first(conn)?;
        let owners = krate.owners(conn)?.into_iter().map(Owner::from).collect();

        Ok(Self { owners })
    }
}

impl Package for Crate {
    fn authors(&self) -> &dyn typomania::AuthorSet {
        self
    }

    fn description(&self) -> Option<&str> {
        // We don't do any checks that require descriptions.
        None
    }

    fn shared_authors(&self, other: &dyn typomania::AuthorSet) -> bool {
        self.owners
            .iter()
            .any(|owner| other.contains(owner.borrow()))
    }
}

impl AuthorSet for Crate {
    fn contains(&self, author: &str) -> bool {
        self.owners.contains(author)
    }
}

/// A representation of an individual owner that can be compared to other owners to determine if
/// they represent the same unique user or team that may own one or more crates.
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
struct Owner(String);

impl Owner {
    fn new(id: i32, kind: i32) -> Self {
        Self(format!("{kind}::{id}"))
    }
}

impl Borrow<str> for Owner {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl From<crate::models::Owner> for Owner {
    fn from(value: crate::models::Owner) -> Self {
        Self::new(value.id(), value.kind())
    }
}

#[cfg(test)]
mod tests {
    use crate::{test_util::pg_connection, typosquat::test_util::Faker};
    use thiserror::Error;

    use super::*;

    #[test]
    fn top_crates() -> Result<(), Error> {
        let mut faker = Faker::new(pg_connection());

        // Set up two users.
        let user_a = faker.user("a")?;
        let user_b = faker.user("b")?;

        // Set up three crates with various ownership schemes.
        let _top_a = faker.crate_and_version("a", "Hello", &user_a, 2)?;
        let top_b = faker.crate_and_version("b", "Yes, this is dog", &user_b, 1)?;
        let not_top_c = faker.crate_and_version("c", "Unpopular", &user_a, 0)?;

        // Let's set up a team that owns both b and c, but not a.
        let not_the_a_team = faker.team("org", "team")?;
        faker.add_crate_to_team(&user_b, &top_b.0, &not_the_a_team)?;
        faker.add_crate_to_team(&user_b, &not_top_c.0, &not_the_a_team)?;

        let mut conn = faker.into_conn();

        let top_crates = TopCrates::new(&mut conn, 2)?;

        // Let's ensure the top crates include what we expect (which is a and b, since we asked for
        // 2 crates and they're the most downloaded).
        assert!(top_crates.contains_name("a")?);
        assert!(top_crates.contains_name("b")?);
        assert!(!(top_crates.contains_name("c")?));

        // a and b have no authors in common.
        let pkg_a = top_crates.get("a")?.unwrap();
        let pkg_b = top_crates.get("b")?.unwrap();
        assert!(!pkg_a.shared_authors(pkg_b.authors()));

        // Now let's go get package c and pretend it's a new package.
        let pkg_c = Crate::from_name(&mut conn, "c")?;

        // c _does_ have an author in common with a.
        assert!(pkg_a.shared_authors(pkg_c.authors()));

        // This should be transitive.
        assert!(pkg_c.shared_authors(pkg_a.authors()));

        // Similarly, c has an author in common with b via a team.
        assert!(pkg_b.shared_authors(pkg_c.authors()));
        assert!(pkg_c.shared_authors(pkg_b.authors()));

        Ok(())
    }

    // It's this or a bunch of unwraps.
    #[derive(Error, Debug)]
    enum Error {
        #[error(transparent)]
        Anyhow(#[from] anyhow::Error),

        #[error(transparent)]
        Box(#[from] Box<dyn std::error::Error>),

        #[error(transparent)]
        Diesel(#[from] diesel::result::Error),
    }
}
