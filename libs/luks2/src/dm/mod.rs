//! Device-mapper support for dm-crypt setup and backing device I/O.

mod abi;
pub(crate) mod crypt;
pub(crate) mod device;
mod table;
