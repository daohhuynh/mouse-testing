/// Minimal stand-in for the `bitflags` crate: a handful of constants and a
/// `bits()` accessor is the whole requirement.
#[macro_export]
macro_rules! bitflags_lite {
    (
        $(#[$outer:meta])*
        pub struct $name:ident: $ty:ty {
            $(
                $(#[$inner:meta])*
                const $flag:ident = $value:expr;
            )*
        }
    ) => {
        $(#[$outer])*
        #[derive(Clone, Copy, PartialEq, Eq, Debug)]
        pub struct $name($ty);

        #[allow(dead_code)]
        impl $name {
            $(
                $(#[$inner])*
                pub const $flag: $name = $name($value);
            )*
            pub const fn bits(self) -> $ty { self.0 }
            #[allow(dead_code)]
            pub const fn contains(self, other: Self) -> bool { self.0 & other.0 == other.0 }
        }

        impl core::ops::BitOr for $name {
            type Output = Self;
            fn bitor(self, rhs: Self) -> Self { $name(self.0 | rhs.0) }
        }
    };
}
