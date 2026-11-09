//! The output sections and the mapping of input sections onto them (ADR 0124 rule 25,
//! `docs/plan/0124-step3-design.md` §5).

use crate::input::{InSection, flags};
use crate::problem::{Problem, Result};

/// An output section, in the order of the layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Out {
    /// `__TEXT,__text`.
    Text,
    /// `__TEXT,__stubs`: one stub per import that a branch calls.
    Stubs,
    /// `__TEXT,__const`.
    TextConst,
    /// `__TEXT,__cstring`.
    Cstring,
    /// `__TEXT,__literal4`.
    Literal4,
    /// `__TEXT,__literal8`.
    Literal8,
    /// `__TEXT,__literal16`.
    Literal16,
    /// `__TEXT,__gcc_except_tab`.
    GccExceptTab,
    /// `__TEXT,__unwind_info`, made by the linker.
    UnwindInfo,
    /// `__TEXT,__eh_frame`.
    EhFrame,
    /// `__DATA_CONST,__got`, made by the linker.
    Got,
    /// `__DATA_CONST,__const`, from the inputs' `__DATA,__const`.
    DataConst,
    /// `__DATA_CONST,__mod_init_func`.
    ModInitFunc,
    /// `__DATA,__data`.
    Data,
    /// `__DATA,__thread_vars`.
    ThreadVars,
    /// `__DATA,__thread_data`.
    ThreadData,
    /// `__DATA,__thread_bss`.
    ThreadBss,
    /// `__DATA,__bss`.
    Bss,
    /// `__DATA,__common`.
    Common,
}

/// Every output section, in layout order.
pub(crate) const ALL: [Out; 19] = [
    Out::Text,
    Out::Stubs,
    Out::TextConst,
    Out::Cstring,
    Out::Literal4,
    Out::Literal8,
    Out::Literal16,
    Out::GccExceptTab,
    Out::UnwindInfo,
    Out::EhFrame,
    Out::Got,
    Out::DataConst,
    Out::ModInitFunc,
    Out::Data,
    Out::ThreadVars,
    Out::ThreadData,
    Out::ThreadBss,
    Out::Bss,
    Out::Common,
];

/// The segments that hold sections.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Seg {
    /// `__TEXT`, r-x.
    Text,
    /// `__DATA_CONST`, rw- and read-only after the fixups.
    DataConst,
    /// `__DATA`, rw-.
    Data,
}

impl Out {
    /// Its segment.
    pub(crate) fn segment(self) -> Seg {
        match self {
            Out::Text
            | Out::Stubs
            | Out::TextConst
            | Out::Cstring
            | Out::Literal4
            | Out::Literal8
            | Out::Literal16
            | Out::GccExceptTab
            | Out::UnwindInfo
            | Out::EhFrame => Seg::Text,
            Out::Got | Out::DataConst | Out::ModInitFunc => Seg::DataConst,
            Out::Data
            | Out::ThreadVars
            | Out::ThreadData
            | Out::ThreadBss
            | Out::Bss
            | Out::Common => Seg::Data,
        }
    }

    /// Its segment's name.
    pub(crate) fn segment_name(self) -> &'static str {
        match self.segment() {
            Seg::Text => "__TEXT",
            Seg::DataConst => "__DATA_CONST",
            Seg::Data => "__DATA",
        }
    }

    /// Its name.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Out::Text => "__text",
            Out::Stubs => "__stubs",
            Out::TextConst | Out::DataConst => "__const",
            Out::Cstring => "__cstring",
            Out::Literal4 => "__literal4",
            Out::Literal8 => "__literal8",
            Out::Literal16 => "__literal16",
            Out::GccExceptTab => "__gcc_except_tab",
            Out::UnwindInfo => "__unwind_info",
            Out::EhFrame => "__eh_frame",
            Out::Got => "__got",
            Out::ModInitFunc => "__mod_init_func",
            Out::Data => "__data",
            Out::ThreadVars => "__thread_vars",
            Out::ThreadData => "__thread_data",
            Out::ThreadBss => "__thread_bss",
            Out::Bss => "__bss",
            Out::Common => "__common",
        }
    }

    /// Its `flags` field: type and attributes.
    pub(crate) fn flags(self) -> u32 {
        match self {
            Out::Text => flags::S_ATTR_PURE_INSTRUCTIONS | flags::S_ATTR_SOME_INSTRUCTIONS,
            Out::Stubs => {
                flags::S_SYMBOL_STUBS
                    | flags::S_ATTR_PURE_INSTRUCTIONS
                    | flags::S_ATTR_SOME_INSTRUCTIONS
            }
            Out::Cstring => flags::S_CSTRING_LITERALS,
            Out::Literal4 => flags::S_4BYTE_LITERALS,
            Out::Literal8 => flags::S_8BYTE_LITERALS,
            Out::Literal16 => flags::S_16BYTE_LITERALS,
            Out::EhFrame => {
                flags::S_COALESCED
                    | flags::S_ATTR_NO_TOC_EH
                    | flags::S_ATTR_STRIP_STATIC_SYMS
                    | flags::S_ATTR_LIVE_SUPPORT
            }
            Out::Got => flags::S_NON_LAZY_SYMBOL_POINTERS,
            Out::ModInitFunc => flags::S_MOD_INIT_FUNC_POINTERS,
            Out::ThreadVars => flags::S_THREAD_LOCAL_VARIABLES,
            Out::ThreadData => flags::S_THREAD_LOCAL_REGULAR,
            Out::ThreadBss => flags::S_THREAD_LOCAL_ZEROFILL,
            Out::Bss | Out::Common => flags::S_ZEROFILL,
            Out::TextConst | Out::GccExceptTab | Out::UnwindInfo | Out::DataConst | Out::Data => {
                flags::S_REGULAR
            }
        }
    }

    /// Whether it has no bytes in the file.
    pub(crate) fn zerofill(self) -> bool {
        matches!(self, Out::ThreadBss | Out::Bss | Out::Common)
    }

    /// Whether the section holds code.
    pub(crate) fn code(self) -> bool {
        matches!(self, Out::Text | Out::Stubs)
    }
}

/// What becomes of an input section.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Class {
    /// Not in the output: debug information, embedded bitcode, an empty section.
    Drop,
    /// `__LD,__compact_unwind`, read into `__unwind_info`.
    CompactUnwind,
    /// `__TEXT,__eh_frame`, split into its records.
    EhFrame,
    /// Placed into an output section.
    Out(Out),
}

/// Where `section` of the object `object` goes.
///
/// # Errors
///
/// U8001 on a section this linker does not know, or one whose type does not match its name.
pub(crate) fn classify(object: &str, section: &InSection<'_>) -> Result<Class> {
    if section.debug() || section.segment == "__LLVM" {
        return Ok(Class::Drop);
    }
    let kind = section.kind();
    let class = match (section.segment.as_str(), section.name.as_str(), kind) {
        ("__LD", "__compact_unwind", flags::S_REGULAR) => Class::CompactUnwind,
        // `ld -r`, which Apple's `strip -S` runs on an object (scripts/release.sh strips the
        // runtime so), writes the section as S_REGULAR; ld64 knows it by its name (#1833).
        ("__TEXT", "__eh_frame", flags::S_COALESCED | flags::S_REGULAR) => Class::EhFrame,
        ("__TEXT", "__text", flags::S_REGULAR) => Class::Out(Out::Text),
        ("__TEXT", "__const", flags::S_REGULAR) => Class::Out(Out::TextConst),
        ("__TEXT", "__cstring", flags::S_CSTRING_LITERALS) => Class::Out(Out::Cstring),
        ("__TEXT", "__literal4", flags::S_4BYTE_LITERALS) => Class::Out(Out::Literal4),
        ("__TEXT", "__literal8", flags::S_8BYTE_LITERALS) => Class::Out(Out::Literal8),
        ("__TEXT", "__literal16", flags::S_16BYTE_LITERALS) => Class::Out(Out::Literal16),
        ("__TEXT", "__gcc_except_tab", flags::S_REGULAR) => Class::Out(Out::GccExceptTab),
        ("__DATA" | "__DATA_CONST", "__const", flags::S_REGULAR) => Class::Out(Out::DataConst),
        ("__DATA" | "__DATA_CONST", "__mod_init_func", flags::S_MOD_INIT_FUNC_POINTERS) => {
            Class::Out(Out::ModInitFunc)
        }
        ("__DATA", "__data", flags::S_REGULAR) => Class::Out(Out::Data),
        ("__DATA", "__thread_vars", flags::S_THREAD_LOCAL_VARIABLES) => Class::Out(Out::ThreadVars),
        ("__DATA", "__thread_data", flags::S_THREAD_LOCAL_REGULAR) => Class::Out(Out::ThreadData),
        ("__DATA", "__thread_bss", flags::S_THREAD_LOCAL_ZEROFILL) => Class::Out(Out::ThreadBss),
        ("__DATA", "__bss", flags::S_ZEROFILL) => Class::Out(Out::Bss),
        ("__DATA", "__common", flags::S_ZEROFILL) => Class::Out(Out::Common),
        _ if section.size == 0 && section.relocs.is_empty() => Class::Drop,
        _ => {
            return Err(Problem::internal(format!(
                "`{object}`: the section {},{} (type {kind:#x}) is not supported by the internal linker",
                section.segment, section.name
            )));
        }
    };
    if let Class::Out(out) = class
        && out.zerofill() != section.zerofill()
    {
        return Err(Problem::internal(format!(
            "`{object}`: the section {},{} has a type that does not match its name",
            section.segment, section.name
        )));
    }
    Ok(class)
}
