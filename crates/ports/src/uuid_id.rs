//! What every id that wraps any UUID has: made from one, read back as one, shown as one.

/// Gives `$id`, a newtype over a [`uuid::Uuid`], `from_uuid`, `as_uuid` and a `Display` that
/// shows the UUID. Its derives stay its own.
macro_rules! uuid_id {
    ($id:ident) => {
        impl $id {
            pub fn from_uuid(uuid: ::uuid::Uuid) -> Self {
                Self(uuid)
            }

            pub fn as_uuid(self) -> ::uuid::Uuid {
                self.0
            }
        }

        impl ::std::fmt::Display for $id {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                ::std::fmt::Display::fmt(&self.0, f)
            }
        }
    };
}

pub(crate) use uuid_id;
