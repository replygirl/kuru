# native-search-paging Specification

## Purpose

Provide bounded, permission-aware native project search and stable line paging
without an external executable or ambient filesystem authority.

## Requirements

### Requirement: Checked native search and paged reading

Kuru SHALL provide native `grep` and `glob` project tools without requiring an
externally installed executable. The tools SHALL use pinned upstream Rust
search components and retain their applicable license notices. Traversal SHALL
remain beneath the retained checked tool root, never follow links, bound files,
bytes, paths, matches, and projected output, and apply recognized-secret
projection before returning results. `grep` SHALL reject binary input rather
than treating it as text. By default, `grep` and `glob` SHALL exclude hidden
paths and paths ignored by the repository's ignore rules; each behavior SHALL
be independently requestable.

`file_read` SHALL accept optional one-based logical UTF-8-text-line `offset`
and positive `limit` values. A paged response SHALL preserve the existing text
content field and add the selected line interval, total line count,
`next_offset` when another line is available, and omitted-line count. Invalid
ranges and binary input SHALL fail clearly without partial paging metadata.

#### Scenario: Search stays within a checked project root
- **WHEN** grep or glob encounters a symbolic link, protected project path, or
  path outside the retained tool root
- **THEN** it skips or rejects that path without reading a target outside the
  root and returns only checked eligible project-relative results.

#### Scenario: Search respects a target denial
- **WHEN** a search request covers a path whose effective native permission
  rule denies that operation
- **THEN** that path's contents and match metadata are not exposed through the
  broader request.

#### Scenario: Search asks only for a discovered candidate
- **WHEN** a discovered grep or glob candidate requires foreground approval
- **THEN** Kuru requests that exact candidate through the operation sender,
  without requesting or granting the supplied search directory as a subtree.

#### Scenario: A caller continues a text page
- **WHEN** file_read is called with a valid offset and limit that do not reach
  the end of a UTF-8 text file
- **THEN** it returns stable logical line units and a next_offset that selects
  the immediately following line on the next call.
