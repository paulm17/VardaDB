use std::fmt;

macro_rules! define_id {
    ($name:ident, $doc:expr) => {
        #[doc = $doc]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(pub u32);

        impl From<u32> for $name {
            fn from(id: u32) -> Self {
                Self(id)
            }
        }

        impl From<usize> for $name {
            fn from(id: usize) -> Self {
                Self(id as u32)
            }
        }

        impl From<$name> for usize {
            fn from(id: $name) -> Self {
                id.0 as usize
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }
    };
}

define_id!(StringId, "ID for interned strings");
define_id!(TypeDefinitionId, "ID for a Type Definition");
define_id!(ObjectDefinitionId, "ID for an Object Definition");
define_id!(InterfaceDefinitionId, "ID for an Interface Definition");
define_id!(FieldDefinitionId, "ID for a Field Definition");
define_id!(EnumDefinitionId, "ID for an Enum Definition");
define_id!(UnionDefinitionId, "ID for a Union Definition");
define_id!(ScalarDefinitionId, "ID for a Scalar Definition");
define_id!(InputObjectDefinitionId, "ID for an Input Object Definition");
define_id!(InputValueDefinitionId, "ID for an Input Value Definition");
define_id!(EnumValueId, "ID for an Enum Value");
define_id!(ResolverDefinitionId, "ID for a Resolver Definition");
