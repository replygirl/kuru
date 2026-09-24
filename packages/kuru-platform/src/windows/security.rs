//! Audited Windows security-descriptor ownership used by files and local pipes.
#![allow(unsafe_code)]

use std::ffi::c_void;
use std::io;
use std::mem::{size_of, zeroed};
use std::os::windows::io::{AsRawHandle, BorrowedHandle, FromRawHandle, OwnedHandle};
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::{ERROR_INSUFFICIENT_BUFFER, INVALID_HANDLE_VALUE, LocalFree};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, GetSecurityInfo,
    SDDL_REVISION_1, SE_FILE_OBJECT, SetSecurityInfo,
};
use windows_sys::Win32::Security::{
    ACCESS_ALLOWED_ACE, ACE_HEADER, ACL, ACL_SIZE_INFORMATION, AclSizeInformation,
    DACL_SECURITY_INFORMATION, EqualSid, GetAce, GetAclInformation, GetSecurityDescriptorControl,
    GetSecurityDescriptorDacl, GetSecurityDescriptorLength, GetTokenInformation, INHERIT_ONLY_ACE,
    INHERITED_ACE, IsValidAcl, IsValidSecurityDescriptor, IsValidSid, IsWellKnownSid,
    OWNER_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION, PSID, SE_DACL_PROTECTED,
    SECURITY_ATTRIBUTES, TOKEN_OWNER, TOKEN_QUERY, TOKEN_USER, TokenOwner, TokenUser,
    UNPROTECTED_DACL_SECURITY_INFORMATION, WinCreatorOwnerRightsSid,
};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_ALL_ACCESS, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_DELETE,
    FILE_SHARE_READ, FILE_SHARE_WRITE, ReOpenFile, WRITE_OWNER,
};
use windows_sys::Win32::System::SystemServices::{ACCESS_ALLOWED_ACE_TYPE, ACCESS_DENIED_ACE_TYPE};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

fn denied(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, message)
}

fn checked_bool(value: i32) -> io::Result<()> {
    if value == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Owns precisely one allocation returned by a documented LocalFree API.
struct LocalMemory(*mut c_void);

impl Drop for LocalMemory {
    fn drop(&mut self) {
        // SAFETY: the allocation came from GetSecurityInfo/SDDL/SID conversion,
        // is uniquely owned here, and is not borrowed beyond this object's life.
        unsafe {
            LocalFree(self.0);
        }
    }
}

struct CurrentUser {
    // usize storage supplies TOKEN_USER/TOKEN_OWNER alignment and keeps SIDs live.
    storage: Vec<usize>,
    bytes: usize,
    default_owner: bool,
}

impl CurrentUser {
    fn read() -> io::Result<Self> {
        Self::read_kind(false)
    }

    fn owner() -> io::Result<Self> {
        Self::read_kind(true)
    }

    fn read_kind(default_owner: bool) -> io::Result<Self> {
        let (kind, minimum) = if default_owner {
            (TokenOwner, size_of::<TOKEN_OWNER>())
        } else {
            (TokenUser, size_of::<TOKEN_USER>())
        };
        let mut token = null_mut();
        // SAFETY: process pseudo-handle is only borrowed; token is an out slot.
        checked_bool(unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) })?;
        // SAFETY: successful OpenProcessToken transfers one owned, non-null handle.
        let token = unsafe { OwnedHandle::from_raw_handle(token) };
        let mut bytes = 0;
        // SAFETY: the null-buffer size query is documented; token remains live.
        let status =
            unsafe { GetTokenInformation(token.as_raw_handle(), kind, null_mut(), 0, &mut bytes) };
        let error = io::Error::last_os_error();
        if status != 0
            || error.raw_os_error() != Some(ERROR_INSUFFICIENT_BUFFER as i32)
            || !(minimum..=65_536).contains(&(bytes as usize))
        {
            return Err(denied("invalid process-token user size"));
        }
        let mut user = Self {
            storage: vec![0; (bytes as usize).div_ceil(size_of::<usize>())],
            bytes: bytes as usize,
            default_owner,
        };
        // SAFETY: aligned storage covers the requested byte count; no pointers
        // into it are retained until GetTokenInformation has finished writing.
        checked_bool(unsafe {
            GetTokenInformation(
                token.as_raw_handle(),
                kind,
                user.storage.as_mut_ptr().cast(),
                bytes,
                &mut bytes,
            )
        })?;
        if bytes as usize > user.bytes || (bytes as usize) < minimum {
            return Err(denied("invalid process-token user result"));
        }
        user.bytes = bytes as usize;
        user.sid()?;
        Ok(user)
    }

    fn sid(&self) -> io::Result<PSID> {
        // SAFETY: read() verifies size and alignment; the buffer is not mutated.
        let sid = unsafe {
            if self.default_owner {
                (*self.storage.as_ptr().cast::<TOKEN_OWNER>()).Owner
            } else {
                (*self.storage.as_ptr().cast::<TOKEN_USER>()).User.Sid
            }
        };
        // SAFETY: storage owns the entire live allocation passed to this check.
        unsafe { bounded_sid(sid, self.storage.as_ptr().cast(), self.bytes)? };
        Ok(sid)
    }

    fn sid_string(&self) -> io::Result<String> {
        let mut string = null_mut();
        // SAFETY: sid() validates a live SID; the output allocation is unique.
        checked_bool(unsafe { ConvertSidToStringSidW(self.sid()?, &mut string) })?;
        let memory = LocalMemory(string.cast());
        if memory.0.is_null() {
            return Err(denied("missing process-token SID string"));
        }
        let mut length = 0;
        // SAFETY: successful conversion guarantees a NUL-terminated string.
        // A valid SID's textual representation is bounded; no arbitrary input
        // pointer or externally supplied string reaches this scan.
        unsafe {
            while *string.add(length) != 0 {
                length += 1;
            }
            String::from_utf16(std::slice::from_raw_parts(string, length))
                .map_err(|_| denied("invalid process-token SID encoding"))
        }
    }
}

/// The caller must keep `buffer..buffer+bytes` allocated and readable. `sid`
/// itself may be invalid; this routine validates its extent before reading it.
unsafe fn bounded_sid(sid: PSID, buffer: *const u8, bytes: usize) -> io::Result<()> {
    let start = buffer as usize;
    let end = start
        .checked_add(bytes)
        .ok_or_else(|| denied("invalid SID allocation"))?;
    let address = sid as usize;
    if address < start
        || address
            .checked_add(8)
            .is_none_or(|header_end| header_end > end)
    {
        return Err(denied("SID lies outside its allocation"));
    }
    // SAFETY: the SID's fixed header lies inside the live allocation; its count
    // is read before validating the complete variable-length SID extent.
    let count = unsafe { *sid.cast::<u8>().add(1) } as usize;
    if address
        .checked_add(8 + count * 4)
        .is_none_or(|sid_end| sid_end > end)
    {
        return Err(denied("SID extends outside its allocation"));
    }
    // SAFETY: the complete SID extent has been checked above.
    if unsafe { IsValidSid(sid) } == 0 {
        return Err(denied("invalid SID"));
    }
    Ok(())
}

/// Retain this value until the native create call using attributes() returns.
pub(crate) struct PrivateSecurity {
    descriptor: LocalMemory,
}

impl PrivateSecurity {
    pub(crate) fn new(access_mask: u32, inherit_children: bool) -> io::Result<Self> {
        let sid = CurrentUser::read()?.sid_string()?;
        let flags = if inherit_children { "OICI" } else { "" };
        // Ordinary child creation inherits the DACL but uses TokenOwner, which
        // can be a group under elevation. OWNER RIGHTS with zero access disables
        // the owner's implicit READ_CONTROL/WRITE_DAC without denying the user's
        // explicit grants. Keep it when sealing an inherited file as well.
        let sddl =
            format!("O:{sid}D:P(A;{flags};0x{access_mask:08x};;;{sid})(A;{flags};0x00000000;;;OW)");
        let string: Vec<_> = sddl.encode_utf16().chain(Some(0)).collect();
        let mut descriptor = null_mut();
        // SAFETY: the generated, NUL-terminated SDDL contains only a validated
        // process SID, fixed syntax and a numeric mask. The result is LocalFree-owned.
        checked_bool(unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                string.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                null_mut(),
            )
        })?;
        if descriptor.is_null() {
            return Err(denied("missing private security descriptor"));
        }
        Ok(Self {
            descriptor: LocalMemory(descriptor),
        })
    }

    pub(crate) fn attributes(&self) -> SECURITY_ATTRIBUTES {
        SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: self.descriptor.0,
            bInheritHandle: 0,
        }
    }

    fn dacl(&self) -> io::Result<*mut ACL> {
        let mut present = 0;
        let mut defaulted = 0;
        let mut dacl = null_mut();
        // SAFETY: the descriptor allocation is retained by self; outputs borrow it.
        checked_bool(unsafe {
            GetSecurityDescriptorDacl(self.descriptor.0, &mut present, &mut dacl, &mut defaulted)
        })?;
        if present == 0 || dacl.is_null() {
            return Err(denied("private descriptor has no DACL"));
        }
        Ok(dacl)
    }
}

/// Validate grants and ownership, returning whether inheritance is protected.
pub(crate) fn private_status(handle: BorrowedHandle<'_>) -> io::Result<bool> {
    let user = CurrentUser::read()?;
    let mut owner = null_mut();
    let mut dacl = null_mut();
    let mut descriptor = null_mut();
    // SAFETY: the borrowed handle stays live; output component pointers borrow
    // the single returned allocation, immediately retained below.
    let status = unsafe {
        GetSecurityInfo(
            handle.as_raw_handle(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            null_mut(),
            &mut dacl,
            null_mut(),
            &mut descriptor,
        )
    };
    if status != 0 {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    let descriptor = LocalMemory(descriptor);
    if descriptor.0.is_null() || dacl.is_null() {
        return Err(denied("private object requires a non-null DACL"));
    }
    // SAFETY: the successful API returned a live security descriptor.
    if unsafe { IsValidSecurityDescriptor(descriptor.0) } == 0 {
        return Err(denied("invalid object security descriptor"));
    }
    // SAFETY: descriptor validity and ownership were established above.
    let bytes = unsafe { GetSecurityDescriptorLength(descriptor.0) } as usize;
    // SAFETY: GetSecurityInfo owns the complete descriptor allocation until drop.
    unsafe { bounded_sid(owner, descriptor.0.cast(), bytes)? };
    // SAFETY: both SID allocations remain live and have been checked.
    let user_owned = unsafe { EqualSid(owner, user.sid()?) } != 0;
    if !user_owned {
        let default_owner = CurrentUser::owner()?;
        // SAFETY: the default-owner query validates and retains its SID too.
        if unsafe { EqualSid(owner, default_owner.sid()?) } == 0 {
            return Err(denied("private object belongs to another user"));
        }
    }
    let mut control = 0;
    let mut revision = 0;
    // SAFETY: control/revision are writable outputs for the retained descriptor.
    checked_bool(unsafe {
        GetSecurityDescriptorControl(descriptor.0, &mut control, &mut revision)
    })?;
    // SAFETY: dacl is a component of the validated retained descriptor.
    if unsafe { IsValidAcl(dacl) } == 0 {
        return Err(denied("invalid object DACL"));
    }
    // SAFETY: zero is a valid initial integer-only ACL_SIZE_INFORMATION value.
    let mut information: ACL_SIZE_INFORMATION = unsafe { zeroed() };
    // SAFETY: the DACL is valid, and output size exactly matches its allocation.
    checked_bool(unsafe {
        GetAclInformation(
            dacl,
            (&mut information as *mut ACL_SIZE_INFORMATION).cast(),
            size_of::<ACL_SIZE_INFORMATION>() as u32,
            AclSizeInformation,
        )
    })?;
    let mut owner_grant = false;
    let mut implicit_owner_rights_suppressed = false;
    for index in 0..information.AceCount {
        let mut ace = null_mut();
        // SAFETY: index is bounded by the native ACL count; ace borrows descriptor.
        checked_bool(unsafe { GetAce(dacl, index, &mut ace) })?;
        let start = descriptor.0 as usize;
        let end = start
            .checked_add(bytes)
            .ok_or_else(|| denied("invalid ACL allocation"))?;
        if (ace as usize) < start
            || (ace as usize)
                .checked_add(size_of::<ACE_HEADER>())
                .is_none_or(|limit| limit > end)
        {
            return Err(denied("ACE header outside descriptor"));
        }
        // SAFETY: ACE_HEADER lies within the retained descriptor, alignment is
        // not assumed when reading potentially unfamiliar native ACE layouts.
        let header = unsafe { ace.cast::<ACE_HEADER>().read_unaligned() };
        let ace_bytes = header.AceSize as usize;
        if ace_bytes < size_of::<ACCESS_ALLOWED_ACE>()
            || (ace as usize)
                .checked_add(ace_bytes)
                .is_none_or(|limit| limit > end)
        {
            return Err(denied("invalid ACE extent"));
        }
        if !matches!(
            u32::from(header.AceType),
            ACCESS_ALLOWED_ACE_TYPE | ACCESS_DENIED_ACE_TYPE
        ) {
            return Err(denied("unmodeled private ACL entry"));
        }
        // SAFETY: these two simple ACE types share the checked fixed layout;
        // the variable SID is separately bounded before any SID API reads it.
        let entry = unsafe { ace.cast::<ACCESS_ALLOWED_ACE>().read_unaligned() };
        let sid = (ace as *mut u8)
            .wrapping_add(std::mem::offset_of!(ACCESS_ALLOWED_ACE, SidStart))
            .cast();
        // SAFETY: the complete ACE extent was checked inside the live descriptor.
        unsafe { bounded_sid(sid, ace.cast(), ace_bytes)? };
        // SAFETY: bounded_sid checked the complete live ACE SID. Only an
        // effective, zero-rights allow ACE is accepted as ownership suppression;
        // inherit-only entries do not protect this object itself.
        if unsafe { IsWellKnownSid(sid, WinCreatorOwnerRightsSid) } != 0 {
            if u32::from(header.AceType) != ACCESS_ALLOWED_ACE_TYPE || entry.Mask != 0 {
                return Err(denied("private object has unsupported owner rights"));
            }
            implicit_owner_rights_suppressed |= u32::from(header.AceFlags) & INHERIT_ONLY_ACE == 0;
        }
        if u32::from(header.AceType) == ACCESS_ALLOWED_ACE_TYPE && entry.Mask != 0 {
            // SAFETY: both SIDs are valid and allocations remain live.
            if unsafe { EqualSid(sid, user.sid()?) } == 0 {
                return Err(denied("private object grants access to another principal"));
            }
            owner_grant |= u32::from(header.AceFlags) & INHERIT_ONLY_ACE == 0;
        }
    }
    if !owner_grant {
        return Err(denied("private object has no effective owner grant"));
    }
    if !user_owned && !implicit_owner_rights_suppressed {
        return Err(denied("private default owner retains implicit access"));
    }
    Ok(control & SE_DACL_PROTECTED != 0)
}

pub(crate) fn require_private(
    handle: BorrowedHandle<'_>,
    require_protected: bool,
) -> io::Result<()> {
    if !private_status(handle)? && require_protected {
        return Err(denied("private root must have a protected DACL"));
    }
    Ok(())
}

pub(crate) fn set_private(handle: BorrowedHandle<'_>, access_mask: u32) -> io::Result<()> {
    require_private(handle, false)?;
    apply_private_dacl(handle, access_mask)
}

fn apply_private_dacl(handle: BorrowedHandle<'_>, access_mask: u32) -> io::Result<()> {
    let security = PrivateSecurity::new(access_mask, false)?;
    // SAFETY: the handle and security allocation remain live for this call;
    // setting only the DACL does not change the already-validated owner.
    let status = unsafe {
        SetSecurityInfo(
            handle.as_raw_handle(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            null_mut(),
            null_mut(),
            security.dacl()?,
            null(),
        )
    };
    if status == 0 {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(status as i32))
    }
}

/// Copy a held regular file's current effective DACL.
/// The caller keeps the destination behind a checked private directory until
/// publication, so broad access does not expose staged payload bytes.
fn canonical_access_token(
    protected: bool,
    owner: &[u8],
    dacl: &[u8],
    used_bytes: usize,
    ace_count: u32,
) -> io::Result<Vec<u8>> {
    if dacl.len() < size_of::<ACL>() || !(size_of::<ACL>()..=dacl.len()).contains(&used_bytes) {
        return Err(denied("file DACL used extent is invalid"));
    }
    // A NULL DACL is refused by file_access_token before this point. Preserve
    // the revision, count, and every ordered ACE byte, but not AclSize's free
    // allocation space or reserved ACL-header padding.
    let ace_bytes = &dacl[size_of::<ACL>()..used_bytes];
    let mut token = Vec::with_capacity(1 + 4 + owner.len() + 1 + 4 + 4 + ace_bytes.len());
    token.push(u8::from(protected));
    token.extend_from_slice(&(owner.len() as u32).to_le_bytes());
    token.extend_from_slice(owner);
    token.push(dacl[0]);
    token.extend_from_slice(&ace_count.to_le_bytes());
    token.extend_from_slice(&(ace_bytes.len() as u32).to_le_bytes());
    token.extend_from_slice(ace_bytes);
    Ok(token)
}

/// Compare a retired regular file's policy after Windows removes its last
/// name. Only the inherited-ACE provenance bit is irrelevant to access on an
/// already-unlinked file; the caller must separately prove the retained file
/// is delete-pending with zero links. Every other ordered ACE byte, including
/// inherit-only/effective status and unknown payloads, remains significant.
pub(crate) fn same_retired_file_access(before: &[u8], after: &[u8]) -> bool {
    fn without_inherited_provenance(token: &[u8]) -> Option<Vec<u8>> {
        let mut token = token.to_vec();
        let owner_len = u32::from_le_bytes(token.get(1..5)?.try_into().ok()?) as usize;
        let owner_end = 5usize.checked_add(owner_len)?;
        let count =
            u32::from_le_bytes(token.get(owner_end + 1..owner_end + 5)?.try_into().ok()?) as usize;
        let ace_len =
            u32::from_le_bytes(token.get(owner_end + 5..owner_end + 9)?.try_into().ok()?) as usize;
        let mut cursor = owner_end.checked_add(9)?;
        if cursor.checked_add(ace_len)? != token.len() || count > ace_len / 4 {
            return None;
        }
        for _ in 0..count {
            let size =
                u16::from_le_bytes(token.get(cursor + 2..cursor + 4)?.try_into().ok()?) as usize;
            let end = cursor.checked_add(size)?;
            if size < 4 || end > token.len() {
                return None;
            }
            token[cursor + 1] &= !(INHERITED_ACE as u8);
            cursor = end;
        }
        (cursor == token.len()).then_some(token)
    }

    match (
        without_inherited_provenance(before),
        without_inherited_provenance(after),
    ) {
        (Some(before), Some(after)) => before == after,
        _ => false,
    }
}

pub(crate) fn file_access_token(source: BorrowedHandle<'_>) -> io::Result<Vec<u8>> {
    let mut owner = null_mut();
    let mut dacl = null_mut();
    let mut descriptor = null_mut();
    // SAFETY: the retained source handle stays live; the returned component
    // pointers borrow the one descriptor allocation retained below.
    let status = unsafe {
        GetSecurityInfo(
            source.as_raw_handle(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            null_mut(),
            &mut dacl,
            null_mut(),
            &mut descriptor,
        )
    };
    if status != 0 {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    let descriptor = LocalMemory(descriptor);
    if descriptor.0.is_null() || owner.is_null() || dacl.is_null() {
        return Err(denied("file access descriptor is incomplete"));
    }
    // SAFETY: GetSecurityInfo returned a complete retained descriptor.
    if unsafe { IsValidSecurityDescriptor(descriptor.0) } == 0 || unsafe { IsValidAcl(dacl) } == 0 {
        return Err(denied("file access descriptor is invalid"));
    }
    // SAFETY: the descriptor was validated and remains allocated.
    let bytes = unsafe { GetSecurityDescriptorLength(descriptor.0) } as usize;
    if !(size_of::<ACL>()..=131_072).contains(&bytes) {
        return Err(denied("file access descriptor exceeds bound"));
    }
    // SAFETY: bounded_sid validates the complete SID extent in this allocation.
    unsafe { bounded_sid(owner, descriptor.0.cast(), bytes)? };
    let sid_len = 8 + (unsafe { *owner.cast::<u8>().add(1) } as usize) * 4;
    let start = descriptor.0 as usize;
    let end = start
        .checked_add(bytes)
        .ok_or_else(|| denied("file access descriptor extent overflow"))?;
    let acl_start = dacl as usize;
    if acl_start < start
        || acl_start
            .checked_add(size_of::<ACL>())
            .is_none_or(|v| v > end)
    {
        return Err(denied("file DACL lies outside its descriptor"));
    }
    // SAFETY: the fixed ACL header is inside the retained descriptor.
    let acl_len = unsafe { (*dacl).AclSize } as usize;
    if acl_len < size_of::<ACL>() || acl_start.checked_add(acl_len).is_none_or(|v| v > end) {
        return Err(denied("file DACL exceeds its descriptor"));
    }
    let mut control = 0;
    let mut revision = 0;
    // SAFETY: the output slots are writable and the descriptor is retained.
    checked_bool(unsafe {
        GetSecurityDescriptorControl(descriptor.0, &mut control, &mut revision)
    })?;
    // SAFETY: IsValidAcl checked the DACL, and the integer-only result has
    // exactly the native structure's writable extent.
    let mut information: ACL_SIZE_INFORMATION = unsafe { zeroed() };
    checked_bool(unsafe {
        GetAclInformation(
            dacl,
            (&mut information as *mut ACL_SIZE_INFORMATION).cast(),
            size_of::<ACL_SIZE_INFORMATION>() as u32,
            AclSizeInformation,
        )
    })?;
    let used_bytes = information.AclBytesInUse as usize;
    if information.AceCount != u32::from(unsafe { (*dacl).AceCount }) {
        return Err(denied("file DACL entry count is inconsistent"));
    }
    // SAFETY: owner and DACL extents were checked against the retained
    // descriptor above; the canonical encoder copies only within those bounds.
    canonical_access_token(
        control & SE_DACL_PROTECTED != 0,
        unsafe { std::slice::from_raw_parts(owner.cast::<u8>(), sid_len) },
        unsafe { std::slice::from_raw_parts(dacl.cast::<u8>(), acl_len) },
        used_bytes,
        information.AceCount,
    )
}

pub(crate) fn copy_file_dacl(
    source: BorrowedHandle<'_>,
    staged: BorrowedHandle<'_>,
) -> io::Result<()> {
    let (source_owner_descriptor, source_owner) = file_owner(source)?;
    // SAFETY: file_owner validated source_owner inside the retained descriptor.
    unsafe { assign_staged_owner(staged, source_owner)? };
    require_same_file_owner(source, staged)?;
    drop(source_owner_descriptor);
    let mut dacl = null_mut();
    let mut descriptor = null_mut();
    // SAFETY: the source handle remains live; the component pointer borrows
    // the single descriptor allocation retained below.
    let status = unsafe {
        GetSecurityInfo(
            source.as_raw_handle(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            null_mut(),
            null_mut(),
            &mut dacl,
            null_mut(),
            &mut descriptor,
        )
    };
    if status != 0 {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    let descriptor = LocalMemory(descriptor);
    if descriptor.0.is_null() || dacl.is_null() {
        return Err(denied("source file has no bounded DACL"));
    }
    // SAFETY: GetSecurityInfo returned a complete retained descriptor.
    if unsafe { IsValidSecurityDescriptor(descriptor.0) } == 0 || unsafe { IsValidAcl(dacl) } == 0 {
        return Err(denied("source file has an invalid DACL"));
    }
    // SAFETY: both file handles and the descriptor stay live throughout the
    // call; only the DACL is copied, never ownership. Protect the staged copy
    // so inheritance from its private parent cannot replace the captured ACEs.
    let status = unsafe {
        SetSecurityInfo(
            staged.as_raw_handle(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            null_mut(),
            null_mut(),
            dacl,
            null(),
        )
    };
    if status == 0 {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(status as i32))
    }
}

fn file_owner(handle: BorrowedHandle<'_>) -> io::Result<(LocalMemory, PSID)> {
    let mut sid = null_mut();
    let mut descriptor = null_mut();
    // SAFETY: the handle stays live and both output pointers borrow the one
    // descriptor allocation retained immediately below.
    let status = unsafe {
        GetSecurityInfo(
            handle.as_raw_handle(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION,
            &mut sid,
            null_mut(),
            null_mut(),
            null_mut(),
            &mut descriptor,
        )
    };
    if status != 0 {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    let descriptor = LocalMemory(descriptor);
    if descriptor.0.is_null() || sid.is_null() {
        return Err(denied("file has no checked owner"));
    }
    // SAFETY: GetSecurityInfo returned the complete retained descriptor.
    if unsafe { IsValidSecurityDescriptor(descriptor.0) } == 0 {
        return Err(denied("invalid file-owner descriptor"));
    }
    // SAFETY: the SID pointer borrows this validated descriptor allocation.
    let bytes = unsafe { GetSecurityDescriptorLength(descriptor.0) } as usize;
    unsafe { bounded_sid(sid, descriptor.0.cast(), bytes)? };
    Ok((descriptor, sid))
}

/// `owner` must be a validated SID retained for the duration of this call.
unsafe fn require_assignable_file_owner(owner: PSID) -> io::Result<()> {
    let user = CurrentUser::read()?;
    let default_owner = CurrentUser::owner()?;
    // SAFETY: callers retain and validate the owner SID; both token queries
    // validate and retain their complete SID allocations.
    if unsafe { EqualSid(owner, user.sid()?) } != 0
        || unsafe { EqualSid(owner, default_owner.sid()?) } != 0
    {
        Ok(())
    } else {
        Err(denied(
            "source file owner is not assignable by the current process token",
        ))
    }
}

/// `source_owner` must be a validated SID retained through this handoff.
unsafe fn assign_staged_owner(staged: BorrowedHandle<'_>, source_owner: PSID) -> io::Result<()> {
    // The original staged handle deliberately lacks WRITE_OWNER. It still
    // proves that this exact object is private before a short-lived reopen adds
    // only the authority needed for the owner handoff.
    require_private(staged, false)?;
    // SAFETY: the caller retains the validated source SID through this call.
    unsafe { require_assignable_file_owner(source_owner)? };
    // SAFETY: ReOpenFile derives the new handle from the retained exact file
    // object, not a pathname. The original movable handle already shares read,
    // write and delete; the reopened handle is immediately RAII-owned.
    let owner_handle = unsafe {
        ReOpenFile(
            staged.as_raw_handle(),
            WRITE_OWNER,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
        )
    };
    if owner_handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: successful ReOpenFile transfers one unique owned handle.
    let owner_handle = unsafe { OwnedHandle::from_raw_handle(owner_handle) };
    // SAFETY: the retained source descriptor owns source_owner through this
    // call; only this exact staged object's owner is changed.
    let status = unsafe {
        SetSecurityInfo(
            owner_handle.as_raw_handle(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION,
            source_owner,
            null_mut(),
            null_mut(),
            null(),
        )
    };
    if status != 0 {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    drop(owner_handle);
    // On elevated tokens, assigning the distinct TokenOwner can normalize an
    // effective zero-mask OWNER RIGHTS ACE into an inherit-only ACE. Restore
    // the identical protected private policy through the retained WRITE_DAC
    // handle before this stage can reach a source-DACL copy or publication.
    apply_private_dacl(staged, FILE_ALL_ACCESS)?;
    // A distinct TokenOwner is accepted only after the staged DACL again has
    // the effective OWNER RIGHTS suppression required by private_status.
    require_private(staged, false)
}

fn require_same_file_owner(
    source: BorrowedHandle<'_>,
    staged: BorrowedHandle<'_>,
) -> io::Result<()> {
    let (source_descriptor, source_sid) = file_owner(source)?;
    let (staged_descriptor, staged_sid) = file_owner(staged)?;
    // SAFETY: both validated SIDs remain inside their retained allocations.
    let same = unsafe { EqualSid(source_sid, staged_sid) } != 0;
    drop((source_descriptor, staged_descriptor));
    if same {
        Ok(())
    } else {
        Err(denied("staged file cannot preserve source owner access"))
    }
}

/// Complete the access-policy handoff after the staged file has moved under
/// its actual destination parent. Protected source ACLs are already final;
/// unprotected sources must regain inheritance from that destination parent.
pub(crate) fn restore_file_dacl_inheritance(
    source: BorrowedHandle<'_>,
    published: BorrowedHandle<'_>,
) -> io::Result<()> {
    let mut dacl = null_mut();
    let mut descriptor = null_mut();
    // SAFETY: the source handle stays live and the DACL borrows the one retained
    // descriptor allocation below.
    let status = unsafe {
        GetSecurityInfo(
            source.as_raw_handle(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            null_mut(),
            null_mut(),
            &mut dacl,
            null_mut(),
            &mut descriptor,
        )
    };
    if status != 0 {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    let descriptor = LocalMemory(descriptor);
    if descriptor.0.is_null() || dacl.is_null() {
        return Err(denied("source file has no bounded DACL"));
    }
    // SAFETY: the complete descriptor and DACL remain owned and live here.
    if unsafe { IsValidSecurityDescriptor(descriptor.0) } == 0 || unsafe { IsValidAcl(dacl) } == 0 {
        return Err(denied("source file has an invalid DACL"));
    }
    let mut control = 0;
    let mut revision = 0;
    // SAFETY: control and revision are writable output slots.
    checked_bool(unsafe {
        GetSecurityDescriptorControl(descriptor.0, &mut control, &mut revision)
    })?;
    if control & SE_DACL_PROTECTED != 0 {
        return Ok(());
    }
    // SAFETY: published is the retained WRITE_DAC file handle after the checked
    // move; setting its unprotected DACL now inherits from the real parent.
    let status = unsafe {
        SetSecurityInfo(
            published.as_raw_handle(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | UNPROTECTED_DACL_SECURITY_INFORMATION,
            null_mut(),
            null_mut(),
            dacl,
            null(),
        )
    };
    if status == 0 {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(status as i32))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs::{Directory, NameRetention, Privacy};
    use std::ffi::OsStr;
    use std::fs::File;
    use std::io::{Read, Seek, Write};
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::{AsHandle, BorrowedHandle};
    use windows_sys::Win32::Security::Authorization::ConvertSecurityDescriptorToStringSecurityDescriptorW;

    fn descriptor(sddl: &str) -> PrivateSecurity {
        let text: Vec<_> = sddl.encode_utf16().chain(Some(0)).collect();
        let mut result = null_mut();
        // SAFETY: fixture-generated SDDL is terminated and result is an out slot.
        checked_bool(unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                text.as_ptr(),
                SDDL_REVISION_1,
                &mut result,
                null_mut(),
            )
        })
        .unwrap();
        PrivateSecurity {
            descriptor: LocalMemory(result),
        }
    }

    #[test]
    fn retired_access_comparison_ignores_only_inherited_ace_provenance() {
        fn token(flags: u8, mask: u32, payload: [u8; 4]) -> Vec<u8> {
            let mut dacl = vec![2, 0, 0, 0, 0, 0, 1, 0];
            dacl.extend_from_slice(&[ACCESS_ALLOWED_ACE_TYPE as u8, flags, 12, 0]);
            dacl.extend_from_slice(&mask.to_le_bytes());
            dacl.extend_from_slice(&payload);
            let size = u16::try_from(dacl.len()).unwrap().to_le_bytes();
            dacl[2..4].copy_from_slice(&size);
            canonical_access_token(false, &[1, 2], &dacl, dacl.len(), 1).unwrap()
        }

        let inherited = token(INHERITED_ACE as u8, 0x12, [3, 4, 5, 6]);
        let explicit = token(0, 0x12, [3, 4, 5, 6]);
        assert!(same_retired_file_access(&inherited, &inherited));
        assert!(same_retired_file_access(&inherited, &explicit));
        assert!(same_retired_file_access(&explicit, &inherited));
        assert!(!same_retired_file_access(
            &inherited,
            &token(INHERIT_ONLY_ACE as u8, 0x12, [3, 4, 5, 6])
        ));
        for flag in [0x01, 0x02, 0x04, 0x20, 0x80] {
            assert!(!same_retired_file_access(
                &inherited,
                &token(flag, 0x12, [3, 4, 5, 6])
            ));
        }
        assert!(!same_retired_file_access(
            &inherited,
            &token(0, 0x13, [3, 4, 5, 6])
        ));
        assert!(!same_retired_file_access(
            &inherited,
            &token(0, 0x12, [3, 4, 5, 7])
        ));
        for offset in [0, 5, 7, 8] {
            let mut changed = explicit.clone();
            changed[offset] ^= 1;
            assert!(!same_retired_file_access(&inherited, &changed));
        }
        for offset in [16, 18] {
            let mut changed = explicit.clone();
            changed[offset] ^= 1; // ACE type or size, respectively.
            assert!(!same_retired_file_access(&inherited, &changed));
        }
        let mut two_aces = explicit.clone();
        two_aces[8..12].copy_from_slice(&2_u32.to_le_bytes());
        two_aces[12..16].copy_from_slice(&24_u32.to_le_bytes());
        two_aces.extend_from_slice(&[ACCESS_ALLOWED_ACE_TYPE as u8, 0, 12, 0]);
        two_aces.extend_from_slice(&0x34_u32.to_le_bytes());
        two_aces.extend_from_slice(&[7, 8, 9, 10]);
        let mut reordered = two_aces.clone();
        reordered[16..28].copy_from_slice(&two_aces[28..40]);
        reordered[28..40].copy_from_slice(&two_aces[16..28]);
        assert!(!same_retired_file_access(&two_aces, &reordered));
        assert!(!same_retired_file_access(
            &inherited,
            &explicit[..explicit.len() - 1]
        ));
    }

    fn set_dacl_on_handle(handle: BorrowedHandle<'_>, dacl: *const ACL) {
        // SAFETY: the fixture retains the exact checked DACL handle and keeps
        // the descriptor alive; null intentionally constructs a null-DACL case.
        let result = unsafe {
            SetSecurityInfo(
                handle.as_raw_handle(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                null_mut(),
                null_mut(),
                dacl,
                null(),
            )
        };
        assert_eq!(
            result,
            0,
            "fixture SetSecurityInfo failed: {}",
            io::Error::from_raw_os_error(result as i32)
        );
    }

    fn set_dacl(file: &File, dacl: *const ACL) {
        use windows_sys::Win32::Storage::FileSystem::{READ_CONTROL, WRITE_DAC};

        // The production source handle intentionally needs only READ_CONTROL.
        // Reopen this retained exact fixture object with the checked read and
        // test-only mutation rights instead of changing production source-open
        // authority.
        // SAFETY: ReOpenFile derives the new handle from this retained object.
        let dacl_handle = unsafe {
            ReOpenFile(
                file.as_raw_handle(),
                READ_CONTROL | WRITE_DAC,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            )
        };
        assert_ne!(
            dacl_handle,
            INVALID_HANDLE_VALUE,
            "fixture ReOpenFile failed: {}",
            io::Error::last_os_error()
        );
        // SAFETY: successful ReOpenFile transfers one unique owned handle.
        let dacl_handle = unsafe { OwnedHandle::from_raw_handle(dacl_handle) };
        set_dacl_on_handle(dacl_handle.as_handle(), dacl);
    }

    fn security_text(file: &File) -> String {
        let mut descriptor = null_mut();
        // SAFETY: fixture handle is live; the one allocated descriptor is retained.
        assert_eq!(
            unsafe {
                GetSecurityInfo(
                    file.as_raw_handle(),
                    SE_FILE_OBJECT,
                    OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
                    null_mut(),
                    null_mut(),
                    null_mut(),
                    null_mut(),
                    &mut descriptor,
                )
            },
            0
        );
        let descriptor = LocalMemory(descriptor);
        let mut text = null_mut();
        let mut length = 0;
        // SAFETY: both descriptor and returned string have scoped RAII owners;
        // the returned native length includes the terminator.
        checked_bool(unsafe {
            ConvertSecurityDescriptorToStringSecurityDescriptorW(
                descriptor.0,
                SDDL_REVISION_1,
                OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
                &mut text,
                &mut length,
            )
        })
        .unwrap();
        let _text = LocalMemory(text.cast());
        // SAFETY: successful conversion provides exactly length UTF-16 units.
        String::from_utf16(unsafe {
            std::slice::from_raw_parts(text, length.saturating_sub(1) as usize)
        })
        .unwrap()
    }

    fn security_shape(file: &File) -> String {
        let user = CurrentUser::read().unwrap();
        let default_owner = CurrentUser::owner().unwrap();
        let mut owner = null_mut();
        let mut dacl = null_mut();
        let mut descriptor = null_mut();
        // SAFETY: the fixture retains the file and the returned descriptor.
        assert_eq!(
            unsafe {
                GetSecurityInfo(
                    file.as_raw_handle(),
                    SE_FILE_OBJECT,
                    OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
                    &mut owner,
                    null_mut(),
                    &mut dacl,
                    null_mut(),
                    &mut descriptor,
                )
            },
            0
        );
        let descriptor = LocalMemory(descriptor);
        let bytes = unsafe { GetSecurityDescriptorLength(descriptor.0) } as usize;
        // SAFETY: the native descriptor owns the returned owner SID.
        unsafe { bounded_sid(owner, descriptor.0.cast(), bytes) }.unwrap();
        assert!(!dacl.is_null());
        assert_ne!(unsafe { IsValidAcl(dacl) }, 0);
        let owner = if unsafe { EqualSid(owner, user.sid().unwrap()) } != 0 {
            "token-user"
        } else if unsafe { EqualSid(owner, default_owner.sid().unwrap()) } != 0 {
            "token-owner"
        } else {
            "other"
        };
        let mut information: ACL_SIZE_INFORMATION = unsafe { zeroed() };
        // SAFETY: GetSecurityInfo returned a live, validated DACL.
        checked_bool(unsafe {
            GetAclInformation(
                dacl,
                (&mut information as *mut ACL_SIZE_INFORMATION).cast(),
                size_of::<ACL_SIZE_INFORMATION>() as u32,
                AclSizeInformation,
            )
        })
        .unwrap();
        let mut aces = Vec::new();
        for index in 0..information.AceCount {
            let mut ace = null_mut();
            // SAFETY: the index is bounded by the native ACL count.
            checked_bool(unsafe { GetAce(dacl, index, &mut ace) }).unwrap();
            let header = unsafe { ace.cast::<ACE_HEADER>().read_unaligned() };
            if !matches!(
                u32::from(header.AceType),
                ACCESS_ALLOWED_ACE_TYPE | ACCESS_DENIED_ACE_TYPE
            ) {
                aces.push(format!(
                    "type={:#04x},flags={:#04x},principal=unmodeled",
                    header.AceType, header.AceFlags
                ));
                continue;
            }
            let entry = unsafe { ace.cast::<ACCESS_ALLOWED_ACE>().read_unaligned() };
            let sid = (ace as *mut u8)
                .wrapping_add(std::mem::offset_of!(ACCESS_ALLOWED_ACE, SidStart))
                .cast();
            // SAFETY: the complete ACE extent is bounded by the retained
            // descriptor before its principal is categorized.
            unsafe { bounded_sid(sid, ace.cast(), usize::from(header.AceSize)) }.unwrap();
            let principal = if unsafe { IsWellKnownSid(sid, WinCreatorOwnerRightsSid) } != 0 {
                "owner-rights"
            } else if unsafe { EqualSid(sid, user.sid().unwrap()) } != 0 {
                "token-user"
            } else if unsafe { EqualSid(sid, default_owner.sid().unwrap()) } != 0 {
                "token-owner"
            } else {
                "other"
            };
            aces.push(format!(
                "type={:#04x},flags={:#04x},mask={:#010x},principal={principal}",
                header.AceType, header.AceFlags, entry.Mask
            ));
        }
        format!("owner={owner};aces=[{}]", aces.join(";"))
    }

    fn owners_equal(left: &File, right: &File) -> bool {
        let (left_descriptor, left_owner) = file_owner(left.as_handle()).unwrap();
        let (right_descriptor, right_owner) = file_owner(right.as_handle()).unwrap();
        // SAFETY: both validated SIDs remain inside the retained descriptors.
        let equal = unsafe { EqualSid(left_owner, right_owner) } != 0;
        drop((left_descriptor, right_descriptor));
        equal
    }

    fn default_owned_file(directory: &Directory) -> (File, bool) {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE};
        use windows_sys::Win32::Storage::FileSystem::{READ_CONTROL, WRITE_DAC};
        // No supplied descriptor: this is ordinary native child creation, as
        // used by an external engine. Only the requested handle rights differ.
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .access_mode(GENERIC_READ | GENERIC_WRITE | READ_CONTROL | WRITE_DAC)
            .open(directory.path().join("ordinary-child"))
            .unwrap();
        let mut owner = null_mut();
        let mut descriptor = null_mut();
        // SAFETY: this fixture holds the file and owns the returned descriptor.
        assert_eq!(
            unsafe {
                GetSecurityInfo(
                    file.as_raw_handle(),
                    SE_FILE_OBJECT,
                    OWNER_SECURITY_INFORMATION,
                    &mut owner,
                    null_mut(),
                    null_mut(),
                    null_mut(),
                    &mut descriptor,
                )
            },
            0
        );
        let descriptor = LocalMemory(descriptor);
        let default_owner = CurrentUser::owner().unwrap();
        let user = CurrentUser::read().unwrap();
        // SAFETY: the native descriptor and queried token allocations remain
        // live; check the returned SID's extent before comparing it.
        unsafe {
            bounded_sid(
                owner,
                descriptor.0.cast(),
                GetSecurityDescriptorLength(descriptor.0) as usize,
            )
            .unwrap();
            assert_ne!(EqualSid(owner, default_owner.sid().unwrap()), 0);
        }
        // SAFETY: both complete SIDs are validated and live.
        let user_owned = unsafe { EqualSid(owner, user.sid().unwrap()) } != 0;
        eprintln!("ordinary native child: TokenOwner equals TokenUser = {user_owned}");
        (file, user_owned)
    }

    #[test]
    fn access_token_ignores_only_unused_acl_storage() {
        let owner = [1, 1, 0, 0, 0, 0, 0, 5];
        let dacl = [
            2, 0, 24, 0, 1, 0, 0, 0, // ACL revision, allocated size, ACE count
            0, 0, 16, 0, 1, 0, 0, 0, // ordered allow ACE and access mask
            1, 1, 0, 0, 0, 0, 0, 5, // SID within that ACE
        ];
        let original = canonical_access_token(false, &owner, &dacl, 24, 1).unwrap();
        let mut extra_capacity = dacl.to_vec();
        extra_capacity[2..4].copy_from_slice(&32u16.to_le_bytes());
        extra_capacity.extend_from_slice(&[0xa5; 8]);
        assert_eq!(
            original,
            canonical_access_token(false, &owner, &extra_capacity, 24, 1).unwrap(),
            "unused ACL allocation bytes changed the access policy token"
        );

        let mut changed_access = dacl;
        changed_access[12] = 2;
        assert_ne!(
            original,
            canonical_access_token(false, &owner, &changed_access, 24, 1).unwrap(),
            "a changed ACE access mask was ignored"
        );
        assert_ne!(
            original,
            canonical_access_token(true, &owner, &dacl, 24, 1).unwrap()
        );
        assert_ne!(
            original,
            canonical_access_token(false, &[1, 1, 0, 0, 0, 0, 0, 6], &dacl, 24, 1).unwrap()
        );
        let empty_dacl = [2, 0, 8, 0, 0, 0, 0, 0];
        assert_ne!(
            original,
            canonical_access_token(false, &owner, &empty_dacl, 8, 0).unwrap()
        );
    }

    #[test]
    fn ordinary_creation_and_sealing_preserve_private_default_owner_rights() {
        let temporary = tempfile::tempdir().unwrap();
        let directory = Directory::ensure_private(&temporary.path().join("private")).unwrap();
        let (mut file, _) = default_owned_file(&directory);
        file.write_all(b"ordinary child remains private").unwrap();
        require_private(file.as_handle(), false).unwrap();
        assert!(security_text(&file).contains(";;;OW)"));
        directory
            .read(std::ffi::OsStr::new("ordinary-child"))
            .unwrap();
        set_private(
            file.as_handle(),
            windows_sys::Win32::Storage::FileSystem::FILE_GENERIC_READ,
        )
        .unwrap();
        require_private(file.as_handle(), true).unwrap();
        assert!(security_text(&file).contains(";;;OW)"));
        assert_eq!(
            std::fs::read(directory.path().join("ordinary-child")).unwrap(),
            b"ordinary child remains private"
        );
    }

    #[test]
    fn default_owner_privacy_checks_effective_zero_rights_without_repair() {
        let temporary = tempfile::tempdir().unwrap();
        let directory = Directory::ensure_private(&temporary.path().join("private")).unwrap();
        let (mut file, user_owned) = default_owned_file(&directory);
        file.write_all(b"unchanged private content").unwrap();
        let sid = CurrentUser::read().unwrap().sid_string().unwrap();
        for (owner_rights, valid_for_user_owner) in [
            ("", true),
            ("(A;IO;0x00000000;;;OW)", true),
            ("(A;;FR;;;OW)", false),
        ] {
            let acl = descriptor(&format!("O:{sid}D:P(A;;FA;;;{sid}){owner_rights}"));
            set_dacl(&file, acl.dacl().unwrap());
            let before = security_text(&file);
            let checked = require_private(file.as_handle(), false);
            // A user owner already has the allowed user's implicit rights.
            // A distinct default group owner must never retain them. Both
            // branches assert their actual native token contract, without
            // requiring privileges or mutating the process token for a fixture.
            assert_eq!(
                checked.is_ok(),
                user_owned && valid_for_user_owner,
                "{owner_rights}: {checked:?}"
            );
            assert_eq!(security_text(&file), before);
            assert_eq!(
                std::fs::read(directory.path().join("ordinary-child")).unwrap(),
                b"unchanged private content"
            );
        }
    }

    #[test]
    fn native_weak_and_null_acls_fail_without_silent_repair() {
        let temporary = tempfile::tempdir().unwrap();
        let directory = Directory::ensure_private(&temporary.path().join("private")).unwrap();
        let mut file = directory.create_new(OsStr::new("record")).unwrap();
        file.write_all(b"private original bytes").unwrap();
        let sid = CurrentUser::read().unwrap().sid_string().unwrap();
        let weak = descriptor(&format!("O:{sid}D:P(A;;FA;;;{sid})(A;;FR;;;WD)"));
        set_dacl(&file, weak.dacl().unwrap());
        let before = security_text(&file);
        assert!(require_private(file.as_handle(), false).is_err());
        assert!(directory.read(OsStr::new("record")).is_err());
        assert_eq!(security_text(&file), before);
        assert_eq!(
            std::fs::read(directory.path().join("record")).unwrap(),
            b"private original bytes"
        );
        set_dacl(&file, null());
        let before = security_text(&file);
        assert!(require_private(file.as_handle(), false).is_err());
        assert_eq!(security_text(&file), before);
    }

    #[test]
    fn private_file_publication_rejects_a_broad_candidate_acl_before_the_move() {
        use crate::fs::{Publication, PublicationPhase};
        let temporary = tempfile::tempdir().unwrap();
        let private = Directory::ensure_private(&temporary.path().join("private")).unwrap();
        let ordinary =
            Directory::open(private.path(), Privacy::Inherited, NameRetention::Movable).unwrap();
        private
            .create_new(OsStr::new("published"))
            .unwrap()
            .write_all(b"private original")
            .unwrap();
        let mut candidate = private.create_new(OsStr::new("candidate")).unwrap();
        candidate.write_all(b"ordinary candidate").unwrap();
        let sid = CurrentUser::read().unwrap().sid_string().unwrap();
        let weak = descriptor(&format!("O:{sid}D:P(A;;FA;;;{sid})(A;;FR;;;WD)"));
        set_dacl(&candidate, weak.dacl().unwrap());
        let before = security_text(&candidate);
        assert!(require_private(candidate.as_handle(), false).is_err());
        let error = private
            .publish_file(
                &ordinary,
                OsStr::new("candidate"),
                &candidate,
                OsStr::new("published"),
                Publication::ReplaceRegular,
            )
            .unwrap_err();
        assert_eq!(error.phase, PublicationPhase::Rejected);
        assert_eq!(
            std::fs::read(private.path().join("published")).unwrap(),
            b"private original"
        );
        assert_eq!(
            std::fs::read(private.path().join("candidate")).unwrap(),
            b"ordinary candidate"
        );
        assert_eq!(security_text(&candidate), before);
    }

    #[test]
    fn shielded_stage_copies_effective_acl_before_checked_publication() {
        use crate::fs::{Publication, copy_file_access, finalize_file_access};
        let temporary = tempfile::tempdir().unwrap();
        let project = temporary.path().join("project");
        std::fs::create_dir(&project).unwrap();
        let parent = Directory::open(&project, Privacy::Inherited, NameRetention::Movable).unwrap();
        let source = parent.create_new(OsStr::new("original")).unwrap();
        let sid = CurrentUser::read().unwrap().sid_string().unwrap();
        let broad = descriptor(&format!(
            "O:{sid}D:P(A;;FA;;;{sid})(A;;FR;;;WD)(A;;FR;;;OW)"
        ));
        set_dacl(&source, broad.dacl().unwrap());
        let original_acl = security_text(&source);

        let private = parent
            .create_private_directory(OsStr::new(".kuru-edit-stage"))
            .unwrap();
        let mut candidate = private.create_new(OsStr::new("payload")).unwrap();
        candidate.write_all(b"published bytes").unwrap();
        let access = copy_file_access(&source, &candidate).unwrap();
        assert!(owners_equal(&source, &candidate));
        private.revalidate().unwrap();
        assert!(private.read(OsStr::new("payload")).is_err());
        assert_eq!(security_text(&candidate), original_acl);
        let stage =
            Directory::open(private.path(), Privacy::Inherited, NameRetention::Movable).unwrap();
        assert_eq!(stage.identity(), private.identity());
        parent
            .publish_file_with_access(
                &stage,
                OsStr::new("payload"),
                &candidate,
                &access,
                OsStr::new("published"),
                Publication::New,
            )
            .unwrap();
        finalize_file_access(&source, &candidate).unwrap();
        assert_eq!(
            std::fs::read(project.join("published")).unwrap(),
            b"published bytes"
        );
        assert_eq!(security_text(&candidate), original_acl);
    }

    #[test]
    fn replacement_retains_private_delete_authority_across_restrictive_target_dacl() {
        use crate::fs::{
            Publication, copy_file_access, finalize_file_access, regular_file_info,
            retained_file_info,
        };
        use windows_sys::Win32::Storage::FileSystem::{DELETE, READ_CONTROL, WRITE_DAC};

        let temporary = tempfile::tempdir().unwrap();
        let project = temporary.path().join("project");
        std::fs::create_dir(&project).unwrap();
        let parent = Directory::open(&project, Privacy::Inherited, NameRetention::Movable).unwrap();
        let mut original = parent.create_new(OsStr::new("replace-me")).unwrap();
        original.write_all(b"old bytes").unwrap();
        let original_identity = regular_file_info(&original).unwrap().identity;
        let sid = CurrentUser::read().unwrap().sid_string().unwrap();
        // The file itself deliberately grants no DELETE. The ordinary parent
        // still grants the current user replacement through DELETE_CHILD.
        let restrictive = descriptor(&format!("O:{sid}D:P(A;;FRFW;;;{sid})"));
        set_dacl(&original, restrictive.dacl().unwrap());
        let original_acl = security_text(&original);

        let stage = parent
            .create_private_directory(OsStr::new(".kuru-edit-restrictive"))
            .unwrap();
        let mut candidate = stage.create_new(OsStr::new("payload")).unwrap();
        candidate.write_all(b"new bytes").unwrap();
        let candidate_identity = regular_file_info(&candidate).unwrap().identity;
        let access = copy_file_access(&original, &candidate).unwrap();
        assert_eq!(security_text(&candidate), original_acl);
        // The private stage parent initially grants DELETE_CHILD, which can
        // independently authorize a later DELETE open despite the file DACL.
        // Remove only that alternative authority in this negative fixture;
        // the retained pre-copy DELETE handle must still publish the candidate.
        let stage_dacl = descriptor(&format!("O:{sid}D:P(A;;FRFW;;;{sid})"));
        let stage_dacl_file = std::fs::OpenOptions::new()
            .read(true)
            .access_mode(READ_CONTROL | WRITE_DAC)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(stage.path())
            .unwrap();
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_ID_INFO, FileIdInfo, GetFileInformationByHandleEx,
        };
        let mut stage_id = std::mem::MaybeUninit::<FILE_ID_INFO>::uninit();
        // SAFETY: FileIdInfo writes the entire correctly sized output record
        // while the directory handle remains live.
        assert_ne!(
            unsafe {
                GetFileInformationByHandleEx(
                    stage_dacl_file.as_raw_handle(),
                    FileIdInfo,
                    stage_id.as_mut_ptr().cast(),
                    std::mem::size_of::<FILE_ID_INFO>() as u32,
                )
            },
            0,
            "fixture could not identify its retained stage directory"
        );
        // SAFETY: the successful native call initialized the complete record.
        let stage_id = unsafe { stage_id.assume_init() };
        let mut stage_identity = [0u8; 24];
        stage_identity[..8].copy_from_slice(&stage_id.VolumeSerialNumber.to_le_bytes());
        stage_identity[8..].copy_from_slice(&stage_id.FileId.Identifier);
        assert_eq!(stage_identity, stage.identity().to_bytes());
        set_dacl_on_handle(stage_dacl_file.as_handle(), stage_dacl.dacl().unwrap());
        // A late handle-only DELETE reopen cannot rely on the parent's
        // DELETE_CHILD grant and must fail after the restrictive DACL copy.
        // SAFETY: candidate is a live retained file handle and no successful
        // handle is expected or adopted by this negative assertion.
        let late_delete = unsafe {
            ReOpenFile(
                candidate.as_raw_handle(),
                DELETE,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                FILE_FLAG_OPEN_REPARSE_POINT,
            )
        };
        if late_delete != INVALID_HANDLE_VALUE {
            // SAFETY: an unexpected successful ReOpenFile still transfers one
            // unique handle which this failing fixture must close.
            drop(unsafe { OwnedHandle::from_raw_handle(late_delete) });
            panic!("restrictive target DACL unexpectedly grants file DELETE");
        }
        let source =
            Directory::open(stage.path(), Privacy::Inherited, NameRetention::Movable).unwrap();
        parent
            .publish_file_with_access(
                &source,
                OsStr::new("payload"),
                &candidate,
                &access,
                OsStr::new("replace-me"),
                Publication::ReplaceRegular,
            )
            .unwrap();
        finalize_file_access(&original, &candidate).unwrap();

        assert_eq!(
            retained_file_info(&original).unwrap().identity,
            original_identity
        );
        original.rewind().unwrap();
        let mut old = Vec::new();
        original.read_to_end(&mut old).unwrap();
        assert_eq!(old, b"old bytes");
        let published = parent.read(OsStr::new("replace-me")).unwrap();
        assert_eq!(
            regular_file_info(&published).unwrap().identity,
            candidate_identity
        );
        assert_eq!(
            std::fs::read(project.join("replace-me")).unwrap(),
            b"new bytes"
        );
        assert_eq!(security_text(&candidate), original_acl);
    }

    #[test]
    fn staged_access_token_rejects_a_held_source_dacl_change() {
        use crate::fs::{copy_file_access, verify_file_access};

        let temporary = tempfile::tempdir().unwrap();
        let parent =
            Directory::open(temporary.path(), Privacy::Inherited, NameRetention::Movable).unwrap();
        let source = parent.create_new(OsStr::new("source")).unwrap();
        let stage = parent
            .create_private_directory(OsStr::new("stage"))
            .unwrap();
        let candidate = stage.create_new(OsStr::new("payload")).unwrap();
        let token = copy_file_access(&source, &candidate).unwrap();
        let sid = CurrentUser::read().unwrap().sid_string().unwrap();
        let changed = descriptor(&format!("O:{sid}D:P(A;;FA;;;{sid})(A;;FR;;;WD)"));
        set_dacl(&source, changed.dacl().unwrap());
        assert!(verify_file_access(&source, &token).is_err());
    }

    #[test]
    fn staged_owner_assignment_rejects_a_non_token_owner_without_changing_the_stage() {
        use windows_sys::Win32::Security::GetSecurityDescriptorOwner;

        let temporary = tempfile::tempdir().unwrap();
        let private = Directory::ensure_private(&temporary.path().join("private")).unwrap();
        let candidate = private.create_new(OsStr::new("payload")).unwrap();
        let before = security_text(&candidate);
        let foreign = descriptor("O:WDD:P(A;;FA;;;WD)");
        let mut owner = null_mut();
        let mut defaulted = 0;
        // SAFETY: the fixture descriptor remains live and output slots are writable.
        checked_bool(unsafe {
            GetSecurityDescriptorOwner(foreign.descriptor.0, &mut owner, &mut defaulted)
        })
        .unwrap();
        let bytes = unsafe { GetSecurityDescriptorLength(foreign.descriptor.0) } as usize;
        // SAFETY: the fixture descriptor remains live and owns the returned SID.
        unsafe { bounded_sid(owner, foreign.descriptor.0.cast(), bytes) }.unwrap();
        // SAFETY: the owner SID was just bounded inside the retained descriptor.
        assert!(unsafe { assign_staged_owner(candidate.as_handle(), owner) }.is_err());
        assert_eq!(security_text(&candidate), before);
        require_private(candidate.as_handle(), true).unwrap();
    }

    #[test]
    fn staged_owner_assignment_retains_effective_owner_rights_privacy() {
        let temporary = tempfile::tempdir().unwrap();
        let private = Directory::ensure_private(&temporary.path().join("private")).unwrap();
        let (source, _) = default_owned_file(&private);
        let candidate = private.create_new(OsStr::new("payload")).unwrap();
        let (source_descriptor, source_owner) = file_owner(source.as_handle()).unwrap();
        let before = security_shape(&candidate);
        // SAFETY: source_descriptor retains the validated source owner SID.
        let assigned = unsafe { assign_staged_owner(candidate.as_handle(), source_owner) };
        let after = security_shape(&candidate);
        assigned.unwrap_or_else(|error| {
            panic!("staged owner assignment failed: {error}; before={before}; after={after}")
        });
        require_same_file_owner(source.as_handle(), candidate.as_handle()).unwrap();
        require_private(candidate.as_handle(), true).unwrap();
        assert!(security_text(&candidate).contains(";;;OW)"));
        drop(source_descriptor);
    }

    #[test]
    fn ordinary_create_and_unprotected_replacement_regain_parent_acl_inheritance() {
        use crate::fs::{Publication, copy_file_access, finalize_file_access, retained_file_info};
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_FLAG_BACKUP_SEMANTICS, READ_CONTROL, WRITE_DAC,
        };

        let temporary = tempfile::tempdir().unwrap();
        let project = temporary.path().join("project");
        std::fs::create_dir(&project).unwrap();
        let parent = Directory::open(&project, Privacy::Inherited, NameRetention::Movable).unwrap();
        let template = parent.create_new(OsStr::new("empty-template")).unwrap();
        let private = parent
            .create_private_directory(OsStr::new(".kuru-edit-stage"))
            .unwrap();
        let mut candidate = private.create_new(OsStr::new("payload")).unwrap();
        candidate.write_all(b"payload").unwrap();
        let default_matches_user = {
            let user = CurrentUser::read().unwrap();
            let default_owner = CurrentUser::owner().unwrap();
            // SAFETY: both token query buffers retain validated SIDs.
            (unsafe { EqualSid(user.sid().unwrap(), default_owner.sid().unwrap()) }) != 0
        };
        assert_eq!(owners_equal(&template, &candidate), default_matches_user);
        let access = copy_file_access(&template, &candidate).unwrap();
        assert!(owners_equal(&template, &candidate));
        let stage =
            Directory::open(private.path(), Privacy::Inherited, NameRetention::Movable).unwrap();
        parent
            .publish_file_with_access(
                &stage,
                OsStr::new("payload"),
                &candidate,
                &access,
                OsStr::new("created"),
                Publication::New,
            )
            .unwrap();
        finalize_file_access(&template, &candidate).unwrap();
        let before = security_shape(&candidate);
        assert_eq!(before, security_shape(&template));
        assert_eq!(file_access_token(candidate.as_handle()).unwrap()[0], 0);

        let parent_acl = std::fs::OpenOptions::new()
            .read(true)
            .access_mode(READ_CONTROL | WRITE_DAC)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(&project)
            .unwrap();
        let sid = CurrentUser::read().unwrap().sid_string().unwrap();
        let changed = descriptor(&format!("O:{sid}D:P(A;OICI;FA;;;{sid})(A;OICI;FR;;;WD)"));
        set_dacl_on_handle(parent_acl.as_handle(), changed.dacl().unwrap());
        let after = security_shape(&candidate);
        assert_ne!(
            after, before,
            "published file stopped inheriting parent changes"
        );
        assert_eq!(
            after,
            security_shape(&template),
            "published file inherited differently from ordinary template"
        );

        let original = parent.create_new(OsStr::new("replace-me")).unwrap();
        let original_identity = crate::fs::regular_file_info(&original).unwrap().identity;
        let replacement_stage = parent
            .create_private_directory(OsStr::new(".kuru-edit-replacement"))
            .unwrap();
        let mut replacement = replacement_stage.create_new(OsStr::new("payload")).unwrap();
        replacement.write_all(b"replacement").unwrap();
        let access = copy_file_access(&original, &replacement).unwrap();
        let replacement_source = Directory::open(
            replacement_stage.path(),
            Privacy::Inherited,
            NameRetention::Movable,
        )
        .unwrap();
        parent
            .publish_file_with_access(
                &replacement_source,
                OsStr::new("payload"),
                &replacement,
                &access,
                OsStr::new("replace-me"),
                Publication::ReplaceRegular,
            )
            .unwrap();
        assert_eq!(
            retained_file_info(&original).unwrap().identity,
            original_identity,
            "retained replaced handle changed identity"
        );
        assert_eq!(
            crate::fs::regular_file_info(&parent.read(OsStr::new("replace-me")).unwrap())
                .unwrap()
                .identity,
            crate::fs::regular_file_info(&replacement).unwrap().identity,
            "published name does not identify the retained replacement"
        );
        finalize_file_access(&original, &replacement).unwrap();
        let replacement_before = security_shape(&replacement);
        assert_eq!(replacement_before, security_shape(&template));
        assert_eq!(file_access_token(replacement.as_handle()).unwrap()[0], 0);

        let changed_again = descriptor(&format!("O:{sid}D:P(A;OICI;FA;;;{sid})"));
        set_dacl_on_handle(parent_acl.as_handle(), changed_again.dacl().unwrap());
        let replacement_after = security_shape(&replacement);
        assert_ne!(
            replacement_after, replacement_before,
            "unprotected replacement stopped inheriting parent changes"
        );
        assert_eq!(
            replacement_after,
            security_shape(&template),
            "unprotected replacement inherited differently from ordinary file"
        );
    }

    #[test]
    fn an_unprotected_owner_only_root_needs_a_validated_private_ancestor() {
        use windows_sys::Win32::Security::UNPROTECTED_DACL_SECURITY_INFORMATION;
        let temporary = tempfile::tempdir().unwrap();
        let directory = Directory::ensure_private(&temporary.path().join("private")).unwrap();
        let security = PrivateSecurity::new(
            windows_sys::Win32::Storage::FileSystem::FILE_ALL_ACCESS,
            true,
        )
        .unwrap();
        // Open a writable ACL handle to this isolated directory. The production
        // checked guard intentionally does not request mutation rights on dirs.
        use std::os::windows::fs::OpenOptionsExt;
        let file = std::fs::OpenOptions::new()
            .read(true)
            .access_mode(
                windows_sys::Win32::Storage::FileSystem::READ_CONTROL
                    | windows_sys::Win32::Storage::FileSystem::WRITE_DAC,
            )
            .custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_BACKUP_SEMANTICS)
            .open(directory.path())
            .unwrap();
        // SAFETY: fixture-only permission mutation of the retained private root;
        // unprotecting deliberately allows its ordinary parent to supply grants.
        assert_eq!(
            unsafe {
                SetSecurityInfo(
                    file.as_raw_handle(),
                    SE_FILE_OBJECT,
                    DACL_SECURITY_INFORMATION | UNPROTECTED_DACL_SECURITY_INFORMATION,
                    null_mut(),
                    null_mut(),
                    security.dacl().unwrap(),
                    null(),
                )
            },
            0
        );
        let before = security_text(&file);
        assert!(require_private(file.as_handle(), true).is_err());
        assert!(
            Directory::open(directory.path(), Privacy::OwnerOnly, NameRetention::Movable).is_err()
        );
        assert_eq!(security_text(&file), before);
    }

    #[test]
    fn a_foreign_system_directory_owner_is_rejected_without_privilege_or_mutation() {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES,
            READ_CONTROL,
        };
        // Read only the public OS directory descriptor. Creating a foreign-owner
        // temp object would require privileges that production and contributor
        // tests do not need. No directory contents or private settings are read.
        let path = std::env::var_os("SystemRoot").expect("native Windows must supply SystemRoot");
        let file = std::fs::OpenOptions::new()
            .read(true)
            .access_mode(FILE_READ_ATTRIBUTES | READ_CONTROL)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)
            .unwrap();
        let before = security_text(&file);
        let error = require_private(file.as_handle(), false).unwrap_err();
        assert!(error.to_string().contains("another user"), "{error}");
        assert_eq!(security_text(&file), before);
    }
}
