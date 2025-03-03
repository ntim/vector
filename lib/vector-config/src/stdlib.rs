use std::{
    collections::{BTreeMap, HashMap, HashSet},
    net::SocketAddr,
    num::{
        NonZeroI16, NonZeroI32, NonZeroI64, NonZeroI8, NonZeroU16, NonZeroU32, NonZeroU64,
        NonZeroU8,
    },
    path::PathBuf,
};

use schemars::{gen::SchemaGenerator, schema::SchemaObject};
use serde::Serialize;
use vector_config_common::validation::Validation;

use crate::{
    schema::{
        finalize_schema, generate_array_schema, generate_bool_schema, generate_map_schema,
        generate_number_schema, generate_optional_schema, generate_set_schema,
        generate_string_schema,
    },
    Configurable, Metadata,
};

// Unit type.
impl Configurable for () {
    fn generate_schema(_: &mut SchemaGenerator, _: Metadata<Self>) -> SchemaObject {
        panic!("unit fields are not supported and should never be used in `Configurable` types");
    }
}

// Null and boolean.
impl<T> Configurable for Option<T>
where
    T: Configurable + Serialize,
{
    fn is_optional() -> bool {
        true
    }

    fn metadata() -> Metadata<Self> {
        // We clone the default metadata of the wrapped type because otherwise this "level" of the schema would
        // effective sever the link between things like the description of `T` itself and what we show for a field of
        // type `Option<T>`.
        //
        // Said another way, this allows callers to use `#[configurable(derived)]` on a field of `Option<T>` so long as
        // `T` has a description, and both the optional field and the schema for `T` will get the description... but the
        // description for the optional field can still be overridden independently, etc.
        T::metadata().convert()
    }

    fn generate_schema(gen: &mut SchemaGenerator, overrides: Metadata<Self>) -> SchemaObject {
        let mut inner_metadata = T::metadata();
        inner_metadata.set_transparent();

        let mut schema = generate_optional_schema(gen, inner_metadata);
        finalize_schema(gen, &mut schema, overrides);
        schema
    }
}

impl Configurable for bool {
    fn generate_schema(gen: &mut SchemaGenerator, overrides: Metadata<Self>) -> SchemaObject {
        let mut schema = generate_bool_schema();
        finalize_schema(gen, &mut schema, overrides);
        schema
    }
}

// Strings.
impl Configurable for String {
    fn generate_schema(gen: &mut SchemaGenerator, overrides: Metadata<Self>) -> SchemaObject {
        let mut schema = generate_string_schema();
        finalize_schema(gen, &mut schema, overrides);
        schema
    }
}

// Numbers.
macro_rules! impl_configuable_numeric {
	($($ty:ty),+) => {
		$(
			impl Configurable for $ty {
				fn generate_schema(gen: &mut SchemaGenerator, overrides: Metadata<Self>) -> SchemaObject {
                    $crate::__ensure_numeric_validation_bounds::<Self>(&overrides);
					let mut schema = generate_number_schema::<Self>();
					finalize_schema(gen, &mut schema, overrides);
					schema
				}
			}
		)+
	};
}

impl_configuable_numeric!(
    u8, u16, u32, u64, usize, i8, i16, i32, i64, isize, f32, f64, NonZeroU8, NonZeroU16,
    NonZeroU32, NonZeroU64, NonZeroI8, NonZeroI16, NonZeroI32, NonZeroI64
);

// Arrays and maps.
impl<T> Configurable for Vec<T>
where
    T: Configurable + Serialize,
{
    fn generate_schema(gen: &mut SchemaGenerator, overrides: Metadata<Self>) -> SchemaObject {
        // We set `T` to be "transparent", which means that during schema finalization, we will relax the rules we
        // enforce, such as needing a description, knowing that they'll be enforced on the field using `HashMap<String,
        // V>` itself, where carrying that description forward to `V` might literally make no sense, such as when `V` is
        // a primitive type like an integer or string.
        let mut element_metadata = T::metadata();
        element_metadata.set_transparent();

        let mut schema = generate_array_schema(gen, element_metadata);
        finalize_schema(gen, &mut schema, overrides);
        schema
    }
}

impl<V> Configurable for BTreeMap<String, V>
where
    V: Configurable + Serialize,
{
    fn is_optional() -> bool {
        // A map with required fields would be... an object.  So if you want that, make a struct
        // instead, not a hashmap.
        true
    }

    fn generate_schema(gen: &mut SchemaGenerator, overrides: Metadata<Self>) -> SchemaObject {
        // We explicitly do not pass anything from the override metadata, because there's nothing to
        // reasonably pass: if `V` is referenceable, using the description for `BTreeMap<String, V>`
        // likely makes no sense, nor would a default make sense, and so on.
        //
        // We do, however, set `V` to be "transparent", which means that during schema finalization,
        // we will relax the rules we enforce, such as needing a description, knowing that they'll
        // be enforced on the field using `BTreeMap<String, V>` itself, where carrying that
        // description forward to `V` might literally make no sense, such as when `V` is a primitive
        // type like an integer or string.
        let mut value_metadata = V::metadata();
        value_metadata.set_transparent();

        let mut schema = generate_map_schema(gen, value_metadata);
        finalize_schema(gen, &mut schema, overrides);
        schema
    }
}

impl<V> Configurable for HashMap<String, V>
where
    V: Configurable + Serialize,
{
    fn is_optional() -> bool {
        // A map with required fields would be... an object.  So if you want that, make a struct
        // instead, not a hashmap.
        true
    }

    fn generate_schema(gen: &mut SchemaGenerator, overrides: Metadata<Self>) -> SchemaObject {
        // We explicitly do not pass anything from the override metadata, because there's nothing to
        // reasonably pass: if `V` is referenceable, using the description for `HashMap<String, V>`
        // likely makes no sense, nor would a default make sense, and so on.
        //
        // We do, however, set `V` to be "transparent", which means that during schema finalization,
        // we will relax the rules we enforce, such as needing a description, knowing that they'll
        // be enforced on the field using `HashMap<String, V>` itself, where carrying that
        // description forward to `V` might literally make no sense, such as when `V` is a primitive
        // type like an integer or string.
        let mut value_metadata = V::metadata();
        value_metadata.set_transparent();

        let mut schema = generate_map_schema(gen, value_metadata);
        finalize_schema(gen, &mut schema, overrides);
        schema
    }
}

impl<V> Configurable for HashSet<V>
where
    V: Configurable + Serialize + Eq + std::hash::Hash,
{
    fn generate_schema(gen: &mut SchemaGenerator, overrides: Metadata<Self>) -> SchemaObject {
        // We explicitly do not pass anything from the override metadata, because there's nothing to reasonably pass: if
        // `V` is referenceable, using the description for `HashSet<V>` likely makes no sense, nor would a default make
        // sense, and so on.
        //
        // We do, however, set `V` to be "transparent", which means that during schema finalization, we will relax the
        // rules we enforce, such as needing a description, knowing that they'll be enforced on the field using
        // `HashSet<V>` itself, where carrying that description forward to `V` might literally make no sense, such as
        // when `V` is a primitive type like an integer or string.
        let mut value_metadata = V::metadata();
        value_metadata.set_transparent();

        let mut schema = generate_set_schema(gen, value_metadata);
        finalize_schema(gen, &mut schema, overrides);
        schema
    }
}

// Additional types that do not map directly to scalars.
impl Configurable for SocketAddr {
    fn referenceable_name() -> Option<&'static str> {
        Some("stdlib::SocketAddr")
    }

    fn description() -> Option<&'static str> {
        Some("An internet socket address, either IPv4 or IPv6.")
    }

    fn generate_schema(gen: &mut SchemaGenerator, overrides: Metadata<Self>) -> SchemaObject {
        // TODO: We don't need anything other than a string schema to (de)serialize a `SocketAddr`,
        // but we eventually should have validation since the format for the possible permutations
        // is well-known and can be easily codified.
        let mut schema = generate_string_schema();
        finalize_schema(gen, &mut schema, overrides);
        schema
    }
}

impl Configurable for PathBuf {
    fn referenceable_name() -> Option<&'static str> {
        Some("stdlib::PathBuf")
    }

    fn description() -> Option<&'static str> {
        Some("A file path.")
    }

    fn metadata() -> Metadata<Self> {
        let mut metadata = Metadata::default();
        if let Some(description) = Self::description() {
            metadata.set_description(description);
        }

        // Taken from
        // https://stackoverflow.com/questions/44289075/regular-expression-to-validate-windows-and-linux-path-with-extension
        // and manually checked against common Linux and Windows paths. It's probably not 100% correct, but it
        // definitely covers the most basic cases.
        const PATH_REGEX: &str = r#"(\/.*|[a-zA-Z]:\\(?:([^<>:"\/\\|?*]*[^<>:"\/\\|?*.]\\|..\\)*([^<>:"\/\\|?*]*[^<>:"\/\\|?*.]\\?|..\\))?)"#;
        metadata.add_validation(Validation::Pattern(PATH_REGEX.to_string()));

        metadata
    }

    fn generate_schema(gen: &mut SchemaGenerator, overrides: Metadata<Self>) -> SchemaObject {
        let mut schema = generate_string_schema();
        finalize_schema(gen, &mut schema, overrides);
        schema
    }
}
