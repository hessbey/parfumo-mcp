# Tool catalog

[← Documentation](README.md)

48 tools, identical in both binaries. Parameters and real outputs for each one are in
[TOOLS.md](TOOLS.md) (in French).

**Kind** — **R**: read. **W**: write, private to the account. **P**: public write, visible
to other members — locked and capped, see [Configuration](CONFIGURATION.md#public-writes).

## Database — no account needed

| Tool | Purpose | Kind |
| --- | --- | --- |
| `parfumo_search_perfumes` | Search by text and filters | R |
| `parfumo_get_perfume` | Full perfume page | R |
| `parfumo_get_perfume_reviews` | Long reviews, paginated and sortable | R |
| `parfumo_get_perfume_statements` | Short reviews (statements) | R |
| `parfumo_get_similar_perfumes` | Similar perfumes, layering suggestions, reported dupes | R |
| `parfumo_get_brand` | A brand and its perfumes | R |
| `parfumo_get_perfumer` | A perfumer and their creations | R |
| `parfumo_get_note` | A fragrance note and the perfumes that use it | R |
| `parfumo_get_rankings` | Rankings by gender, new releases, trends, by year | R |
| `parfumo_fetch_page` | Fallback: cleaned text of a Parfumo page (GET, Parfumo domain only) | R |

## Collection

| Tool | Purpose | Kind |
| --- | --- | --- |
| `parfumo_list_collections` | The account's lists (have, want, had, custom…) | R |
| `parfumo_get_collection` | Contents of one list | R |
| `parfumo_add_to_collection` | Add a perfume to a list | W |
| `parfumo_move_collection_item` | Move an item to another list | W |
| `parfumo_update_collection_item` | Edit an item's details (quantity, bottle size, price paid, remaining volume, private note, custom columns) | W |
| `parfumo_remove_from_collection` | Remove an item | W |
| `parfumo_manage_custom_collection` | Create, rename, hide or delete a custom list | W |
| `parfumo_export_collection` | CSV export | R |
| `parfumo_collection_stats` | Statistics over the collection | R |

## Wear log

| Tool | Purpose | Kind |
| --- | --- | --- |
| `parfumo_log_wear` | Record that a perfume was worn | P |
| `parfumo_list_wears` | Wear history | R |
| `parfumo_delete_wear` | Delete a wear | W |

## Ratings and reviews

| Tool | Purpose | Kind |
| --- | --- | --- |
| `parfumo_rate_perfume` | Rate scent, longevity, sillage, bottle, value | P |
| `parfumo_classify_perfume` | Vote on type, season, occasion… | P |
| `parfumo_list_my_ratings` | The account's ratings | R |
| `parfumo_write_review` | Publish or replace a review | P |
| `parfumo_write_statement` | Publish or replace a statement | P |
| `parfumo_delete_review_or_statement` | Delete one | W |
| `parfumo_list_my_reviews` | The account's reviews and statements | R |

## Social

| Tool | Purpose | Kind |
| --- | --- | --- |
| `parfumo_get_user` | A member's public profile, one of their lists, their reviews or statements | R |
| `parfumo_list_messages` | Conversations in the inbox | R |
| `parfumo_read_message_thread` | Messages of one conversation | R |
| `parfumo_send_message` | Send a private message | P |
| `parfumo_get_notifications` | Pending notifications | R |
| `parfumo_manage_follow` | List follows; follow or unfollow a member | R / P |
| `parfumo_wall` | Read a wall; post on a wall | R / P |
| `parfumo_forum_browse` | Categories, threads of a category, search | R |
| `parfumo_forum_read_thread` | Posts of a thread | R |
| `parfumo_forum_post` | Reply in a thread or open one | P |
| `parfumo_blog` | List and read articles; submit an article | R / P |
| `parfumo_souk_browse` | Offers, requests and splits on the Souk; one entry | R |
| `parfumo_souk_manage_offer` | Create or edit a Souk entry (removal is not supported) | P |
| `parfumo_communities` | Community directory, posts, one post (read-only) | R |

## Profile and account

| Tool | Purpose | Kind |
| --- | --- | --- |
| `parfumo_get_my_profile` | Profile, points, counters, preferences, gender, country, time zone | R |
| `parfumo_update_profile` | Gender, country, time zone, scent preferences, signature scent | P |
| `parfumo_get_settings` | Notification, privacy and site settings | R |
| `parfumo_update_settings` | Change one setting | W |
| `parfumo_session_status` | State of the account link and remaining daily quotas | R |

## Conventions

- **Schemas.** JSON Schema for inputs and outputs, generated from the Rust types. Each
  result carries `structuredContent` and a short readable text.
- **Annotations.** `readOnlyHint` and `destructiveHint` are filled in honestly.
- **Writes.** Every write tool accepts `dry_run`, is idempotent where the site allows it,
  and reads the real state back after writing.
- **Pagination.** `limit` (25 by default, 100 at most) and `cursor`.
- **Size.** Responses are capped at about 25,000 characters.
- **Third-party content.** Text written by members is placed under
  `untrusted_member_content` and flagged as untrusted, so an assistant does not take it for
  instructions.

## Errors

Errors come back with `isError` and a code that tells the assistant what to do next.

| Code | Meaning |
| --- | --- |
| `not_found` | No such perfume, member, thread… |
| `invalid_input` | A parameter is wrong; the message says which |
| `not_linked` | The tool needs a Parfumo account and none is linked |
| `reauth_required` | The session expired and could not be renewed; link the account again |
| `captcha_required` | Parfumo asked for a human check; switch to cookie mode |
| `rate_limited` | Parfumo throttled the request, or a daily public-write cap is reached; the message gives the delay |
| `parser_outdated` | The page no longer has the expected structure; the site has changed |
| `upstream_error` | Parfumo is unreachable or answered with an error |
