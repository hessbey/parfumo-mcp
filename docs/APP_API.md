# The application interface of Parfumo (`/app/v1`)

[← Documentation](README.md)

Parfumo's own application talks to the site through a JSON interface under
`https://www.parfumo.com/app/v1/`. This page records what is known about it, as a basis for
moving the tools from HTML parsing to it. Nothing here is used by the code yet.

**How it was found.** The parameters come from reading the JavaScript of the application
(version 5.8.2), not from running it. The response shapes come from 55 read-only requests
made on 13 November 2026 with the session token of the test account, two seconds apart.
No write was sent.

## Rules

- **Only the user's own session token.** The application also carries a key of its own, sent
  for public data. This project never uses it: every call below was made with the token
  obtained by logging in with the account's own e-mail and password, and public data
  answered just as well. Consequence: through this interface, even public data needs a
  linked account.
- **No disguise.** The application sends `X-App-Version`, `X-Platform`, `X-Install-Id` and
  `X-Language`. None is required: the calls were made with an honest `User-Agent` and
  nothing else.
- **A refused token is never retried.** Two requests without a valid token were followed by
  a block of the address ("Access Denied … try again in 5 minutes", status 404, an HTML
  page). A 401 must stop the calls for that account until it is linked again.
- **Publicly visible writes** keep the safeguards of the current tools, and text is never
  written by a model.

## Requests

```
<GET|POST> https://www.parfumo.com/app/v1/<path>?<query>
Authorization: Bearer <session token>
Content-Type: application/x-www-form-urlencoded; charset=UTF-8     (POST with a body)
```

Arrays in a body are sent as `key[]=a&key[]=b`. Files (message attachments, photo search) go
as `multipart/form-data`. Answers are JSON. Without a valid token the answer is 401 with
`{"status":"error","message":"Authorization header required"}` or
`"Authentication required"`, whether the header is missing or the token is dead.

## Session

| Call | Body | Answer |
| --- | --- | --- |
| `POST /auth/login`, no `Authorization` | `email` (the e-mail or the nickname: both were accepted), `password`, `device_id` (any text) | `{status, auth_token, user_id, nick_name, photo}` |
| `GET /auth/me` | — | `{status, user: {id, nick_name, email, gender, time_zone_id, …}}` |
| `POST /auth/logout` | — | ends the session: the token stops working |

The token is separate from the `pf_auth` cookie of the web site. Whether the cookie is
accepted here is not known.

## Calls

`GET` parameters go in the query, `POST` ones in the body. *Seen* tells whether the answer
was recorded; its shape is in the appendix.

### Database

| Call | Parameters | Seen |
| --- | --- | --- |
| `GET /search` | `filter` (the words), `pin` | yes |
| `GET /perfume_details/{id}` | — | yes |
| `GET /similar` | `action=list`, `type=similar`, `p_id`, `limit` | yes |
| `GET /similar` | `action=detail`, `type`, `p_id`, `s_id` | yes |
| `GET /similar` | `action=search`, `type`, `p_id`, `query` | no |
| `GET /rating_details/{id}/{scent\|durability\|sillage\|bottle\|pricing}` | `offset`, `sort` | yes |
| `GET /brand_details` | `b_id`, `page`, `per_page`, `o`, `g_f`, `g_m`, `g_u` | yes |
| `GET /perfumer_details` | `pf_id`, then as above | yes |
| `GET /fragrance_notes` | `feature=overview` | yes |
| `GET /fragrance_notes` | `feature=search`, `q` | yes |
| `GET /fragrance_notes` | `feature=note`, `note_id`, `page`, `per_page`, `gender`, `pyramid` | yes |
| `GET /fragrance_notes` | `feature=category`, `cat_id`, `page`, `per_page`, `gender` | yes |
| `GET /fragrance_notes` | `feature=combine`, …, `page`, `per_page` | no |
| `GET /release_years` | — | yes |
| `GET /perfumes` | `y_0` (year), `b_id` (brand), `page`, `per_page`, `o`, `g_f`, `g_m`, `g_u` : filters combine | yes |
| `GET /discover` | `feature=tops_flops`, `type`, `gender`, `period` | yes |
| `GET /discover` | `feature=by_color`, `color`, `gender` | no |
| `GET /discover/{id}` | — | yes |
| `GET /prices/{id}` | `provider`, `order`, `f` | yes |
| `GET /compare/{a}/{b}`, `GET /compare_suggestions/{id}` | — | yes |
| `GET /photos/{id}` | — | yes |
| `GET /pronunciation/{id}` | `type` (value not found: 404 with `perfume`) | no |
| `GET /home` | `app_version` | yes |
| `GET /blog_details` | `name_url` | no |
| `GET /barcode`, `POST /barcode`, `POST /photo_search`, `POST /smell_it`, `GET /consultant` | — | no |

### Collection

| Call | Parameters | Seen |
| --- | --- | --- |
| `GET /wardrobe` | — : the lists and their counts | yes |
| `GET /wardrobe` | `p_id` : where one perfume is, personal note, whether rated | yes |
| `GET /wardrobe` | `type` (a list), `order`, `remember` | yes |
| `GET /wardrobe` | `overview=1`, `per`, `if_version` | yes |
| `POST /wardrobe` | `type=add`, `p_id`, `wr` (the list) | no |
| `POST /wardrobe` | `type=remove`, `p_id`, `wr` | no |
| `POST /wardrobe` | `type=move`, `p_id`, `wr`, `to` | no |
| `POST /wardrobe` | `type=reorder`, `wr`, `order` (JSON array of ids) | no |
| `POST /wardrobe` | `type=update_details`, `wr_id`, `wr`, `bottle_size`, `paid`, `currency`, `quantity`, `date_added`, `ml` | no |
| `POST /wardrobe` | `type=update_note`, `wr_id`, `wr`, `note` | no |
| `POST /wardrobe/custom` | `action=create`, `name` | no |
| `POST /wardrobe/custom` | `action=update_name\|update_icon\|delete`, `collection_id`, `name` or `icon` | no |
| `POST /wardrobe/custom` | `action=reorder`, `order` (JSON array) | no |
| `GET /shelfcontrol` | `list` | yes |
| `GET /wardrobe_profiler` | `type`, `section`, `notes_type` (section names not found) | no |
| `GET /notes_list` | `page`, `per_page` : personal notes | yes |
| `POST /notes` | `p_id`, `action=add\|edit\|delete`, `text` | no |

### Wear log

| Call | Parameters | Seen |
| --- | --- | --- |
| `GET /tracker` | — : what is worn today, last worn | yes |
| `POST /tracker` | `action=set_iwear`, `perfume_id` ; `action=reset_iwear` | no |
| `GET /tracker_overview` | `offset`, `limit` : the history | yes |
| `POST /tracker_overview` | `action=delete\|update_layering\|update_comment\|update_date`, `l_id`, and `layered_with_id`, `comment` or `wear_date` | no |
| `GET /tracker_calendar` | `y`, `m` | yes |
| `POST /tracker_calendar` | `action=log`, `p_id`, `wear_date` | no |
| `GET /tracker_stats` | — | yes |
| `GET /sotd` | — | yes |

### Ratings, reviews, comments

| Call | Parameters | Seen |
| --- | --- | --- |
| `GET /rating` | `p_id` : the member's five ratings, out of 100 | yes |
| `POST /rating` | `p_id`, `rating_type` (one of the five), `rating_value` | no |
| `GET /classification` | `p_id` : the groups, their options, what was voted | yes |
| `POST /classification` | `p_id`, `c`, `ct`, `action=add\|remove` | no |
| `GET /reviews/{id}`, `GET /statements/{id}` | `page`, `per_page`, `o` | yes |
| `POST /reviews`, `POST /statements` | the text and its fields (not read in detail) | no |
| `GET /comments/{reviews\|statements}/{id}` | `page`, `per_page` | yes |
| `POST /comments` | `type`, `item_id`, `reply_to_id`, `text` ; or `action=delete`, `id` | no |
| `POST /award` | `type`, `item_id`, `state=1\|0` | no |
| `GET /translate/{reviews\|statements}/{id}` | `lang` | no |

### Members and messages

| Call | Parameters | Seen |
| --- | --- | --- |
| `GET /users/{nick}` | — | yes |
| `GET /users/{nick}/collection` | `type` | yes |
| `GET /users/{nick}/reviews`, `/statements` | `page`, `per_page` | yes |
| `GET /users/{nick}/followers`, `/following` | `page`, `per_page`, `order` | yes |
| `GET /users/{nick}/photos` | — | yes |
| `POST /users/{nick}/follow` | `action=follow\|unfollow` | no |
| `GET /user_search` | `q`, `limit` | yes |
| `GET /feed` | `page`, `per_page` | yes |
| `GET /postbox` | `from`, `filter=new` | yes (empty) |
| `GET /postbox/unread`, `GET /postbox/search` (`q`) | — | yes |
| `GET /postbox/{nick}` | `from` : one conversation | no |
| `POST /postbox/{nick}` | the message | no |
| `POST /postbox/message/delete` (`msg_id`), `/postbox/conversation/delete` (`nick`), `/postbox/{nick}/block` (`action`) | — | no |
| `GET /timezones` | — | yes |
| `POST /account` | `action=update_gender`, `gender` ; `action=update_timezone`, `time_zone_id` | no |

**Never to be wired:** `POST /account` with `action=pause`, `delete` or `register`, and the
password and activation calls.

## What the interface does not cover

The forum, the Souk, the communities, notifications, the wall, the CSV export of a
collection and most account settings (e-mail, password, website, personal details). These
stay on the web pages.

## Observations

- `GET /perfumes` is the filtered listing: `b_id` works alone (104 perfumes of one brand)
  and together with `y_0` (1 match). `y_1` was ignored: a range of years is not known to
  exist. Its rows are documents of the search index, with many more fields than the rows of
  `GET /search` (notes by id, votes, classification counters), under other names
  (`p_name`, `p_release_year`, `b_name_url`…).
- Sort orders (`o`) read in the application: `nr_desc`, `v_desc`, `n_asc`, `n_desc`.
- `GET /search` answers with up to 20 perfumes plus the brands and notes that match the
  words; it takes no filter.

- Values are often numbers written as text (`"p_id": "9438"`), and sometimes plain numbers
  for the same field in another answer.
- Genders come in German whatever the language: `maennlich`, `weiblich`, `unisex`.
- Answers about a perfume mix public data with the caller's own (`wr`, `wr_lists`,
  `voted`): they cannot be put in a cache shared by several users as they are.
- Reviews came back in several languages; the effect of `X-Language` was not measured.
- The collection, the wear log, the ratings and the personal notes were recorded a second
  time once the test account held a few entries (appendix, second part). Messages have only
  been seen as empty lists.

## Open questions

1. Does the `pf_auth` cookie open this interface, or only a token from `/auth/login`?
2. How long does a token live, and does a web login end it?
3. Shapes of the non-empty answers of the account, and every write.
4. Does `X-Language` change names and texts?
5. The limits of the site on this interface, beyond the block that follows refused tokens.

## Appendix: shapes recorded

Types only, no value. `number-as-text` is a number sent as a string; `[x, …]` a list.

`GET /auth/me`

```
{status: text, user: {id: number-as-text, nick_name: text, email: text, permissions: number-as-text, user_level: number-as-text, last_login: number-as-text, last_mobile_login: number-as-text, photo: text, gender: text, time_zone_id: number, cover: text}}
```

`GET /search?filter=sauvage`

```
{notes: [], brands: [], similar_queries: bool, perfumes: [{p_id: number-as-text, p_name: text, con_ext_name: text, con_ext_concentration: text, p_name_url: text, url_image: text, release_year: number-as-text, gender: text, b_id: number-as-text, b_name: text, b_name_url: text}, …]}
```

`GET /perfume_details/9438`

```
{b: {id: number-as-text, manufacturer_id: number-as-text, country_id: number-as-text, name: text, names_previous: null, names_alternative_language: null, name_url: text, name_sound_override: text, name_sound_update: number-as-text, name_sound_lang: text, name_sound_file: text, website: text, instagram: text, founding_year: number-as-text, nr_of_perfums: number-as-text, tags: text, avg_voting: number-as-text, avg_voting_month: number-as-text, total_votings: number-as-text, total_votings_month: number-as-text, in_wardrobes: number-as-text}, p: {id: number-as-text, brand_id: number-as-text, manufacturer_id: number-as-text, release_year: number-as-text, status: text, name: text, name_url: text, name_sound_override: text, name_sound_update: number-as-text, name_sound_file: text, name_sound_lang: text, gender: text, top_notes: [{…}, …], middle_notes: [{…}, …], base_notes: [{…}, …], notes: [{…}, …], perfumer1_id: number-as-text, perfumer2_id: number-as-text, perfumer3_id: number-as-text, perfumer4_id: number-as-text, bottle_designer: text, limited_edition: text, seasonal: text, bottle_edition: text, concentration_edition: text, ce_perfume_id: number-as-text, be_perfume_id: number-as-text, pic_name: text, total_votings: number-as-text, total_votings_dur: number-as-text, total_votings_sillage: number-as-text, total_votings_bottle: number-as-text, total_votings_pricing: number-as-text, avg_voting: number, avg_voting_dur: number, avg_voting_sillage: number, avg_voting_bottle: number, avg_voting_pricing: number, voting_distribution: text, voting_distribution_dur: text, voting_distribution_sillage: text, voting_distribution_bottle: text, voting_distribution_pricing: text, total_reviews: number, total_reviews_en: number-as-text}, i: [text, …], c_total: [number, …], c: [[{…}, …], …], c_poles: [{ct: number, n: text, c: text, cd: text}, …], wr: [], wr_lists: {collection_buttons: [{…}, …]}}
```

`GET /similar?action=list&type=similar&p_id=9438&limit=10`

```
{status: text, perfumes: [{con_ext_name: text, con_ext_concentration: text, id: number-as-text, p_name: text, b_name: text, release_year: number-as-text, gender: text, url_image: text, wilson_score: number-as-text, percentage: number, cnt_yes: number, cnt_no: number, cnt: number}, …], total: number, limit: number}
```

`GET /rating_details/9438/scent`

```
{status: text, type: text, total: number, avg: number, avg_text: text, distribution: {<number>: number, …}, following_count: number, sort: text, offset: number, has_more: bool, users: [{id: number, nick_name: text, photo: text, rating: number, date: text}, …]}
```

`GET /fragrance_notes?feature=overview`

```
{popular: [{id: number, name: text, nr: number, img: text}, …], categories: [{id: number, name: text, name_url: text, nr_of_perfumes: number, sub_categories: [{…}, …], top_notes: [{…}, …]}, …]}
```

`GET /fragrance_notes?feature=search&q=vanilla`

```
{notes: [{id: number, name: text, nr: number, img: text}, …], categories: [{id: number, name: text, nr: number, img: text, imgs: [text, …]}, …]}
```

`GET /perfumes?y_0=2020&page=1&per_page=20`

```
{matches: number, rows: [{p_name: text, p_total_reviews_de: number-as-text, b_name_url: text, cla_37: number-as-text, cla_8: number-as-text, p_wilson_score_bottle: number-as-text, cla_21: number-as-text, p_in_wardrobes: number-as-text, p_middle_notes: text, m_id: number-as-text, cla_22: number-as-text, cla_18: number-as-text, hsv_s: number-as-text, hsv_h: number-as-text, p_total_votings: number-as-text, cla_32: number-as-text, hsv_v: number-as-text, cla_12: number-as-text, cla_5: number-as-text, p_total_reviews_en: number-as-text, p_gender: text, p_base_notes: text, c_type: text, cla_17: number-as-text, cla_13: number-as-text, p_total_votings_dur: number-as-text, c_name_url: text, p_bottle_edition: text, p_nc_p_middle_notes: text, cla_38: number-as-text, cla_1: number-as-text, p_avg_voting_pricing: number-as-text, p_nc_p_notes_all: text, cla_15: number-as-text, p_name_url: text, p_status: text, p_notes: text, cla_34: number-as-text, cla_26: number-as-text, cla_19: number-as-text, p_wilson_score_dur: number-as-text, p_total_votings_sillage: number-as-text, p_avg_voting_bottle: number-as-text, cla_31: number-as-text, p_total_votings_pricing: number-as-text}, …]}
```

`GET /prices/9438`

```
{products: [{prog_id: number-as-text, prog_name: text, prog_url: text, prog_priority: number-as-text, prod_name: text, prod_url: text, prod_currency: text, prod_price: number-as-text, prod_price_old: number-as-text, prod_price_shipping: number-as-text, prod_image_src: text, prod_pic_name: text, cdn: number-as-text, prog_image_src: text, prog_image_src_dark: text, prod_price_formatted: number-as-text, prod_price_shipping_formatted: number-as-text, prod_content: number, prod_unit: text, prod_content_unit: text, price_per_unit: text}, …], filters: {contents: [text, …], content_counts: {240 ml: number, 30 ml: number, 50 ml: number, 100 ml: number}}}
```

`GET /home?app_version=5.8.2`

```
{remarkable_perfumes: [{p_id: number-as-text, p_name: text, p_name_url: text, p_concentration: text, p_gender: text, p_release_year: number-as-text, b_id: number-as-text, b_name: text, url_image_240: text, url_image_480: text, url_image_720: text}, …], blog_teasers: [{id: number-as-text, number: number-as-text, title: text, subtitle: text, url: text, img: text, img_alt: text, lang: text, name_url: text, url_image_1000: text, url_image_1000_webp: text}, …], app_version: {current: text, minimum_supported: number-as-text, update_available: bool, force_update: bool}, rows: [{p_id: number-as-text, p_name: text, p_name_url: text, p_concentration: text, p_gender: text, p_release_year: number-as-text, b_id: number-as-text, b_name: text, url_image_240: text, url_image_480: text, url_image_720: text}, …], pagination: {total: number, per_page: number, current_page: number, last_page: number}}
```

`GET /reviews/9438?page=1&per_page=20`

```
{reviews: [{voting: number-as-text, voting_r: number, awards: number-as-text, replies: number, review_id: number-as-text, user_id: number, awarded: number, awards_hidden: number, photo: text, nick_name: text, gender: text, date: text, review_title: text, review_text: text}, …]}
```

`GET /statements/9438?page=1&per_page=20`

```
{statements: [{voting: number-as-text, voting_r: number, awards: number-as-text, replies: number, statement_id: number-as-text, user_id: number, awarded: number, photo: text, nick_name: text, gender: text, date: text, statement_text: text}, …]}
```

`GET /users/<own nick>`

```
{user: {id: number, nick_name: text, photo: text, cover: text, gender: text, country_id: number, country: text, country_code: text, reg_date: number, scores: number, followers: number-as-text, following: number-as-text, is_own: number, is_following: number, can_message: number, wearing: null, stats: {reviews: number, statements: number, photos: number}, collection: {have: number, had: number, want: number, tested: number, memo: number, filling: number, mini: number}, lists: []}}
```

`GET /users/<own nick>/collection?type=have`

```
{type: text, perfumes: []}
```

`GET /wardrobe?overview=1&per=20`

```
{login_status: number, nick_name: text, user_id: number-as-text, version: number-as-text, per: number, collections: []}
```

`GET /wardrobe?type=have`

```
{login_status: number, nick_name: text, user_id: number-as-text, wr: [], list_details: {id: number, name: text, items: null, order: number, icon: text, hidden: text, notes: text}, url_web: text, order: text}
```

`GET /wardrobe?p_id=9438`

```
{login_status: number, nick_name: text, user_id: number-as-text, collection_buttons: [{type: text, icon: text, name: text, nr: number-as-text, in_wr: number}, …], url_web: text, note: {id: number, text: text, date: number}, has_rating: bool, has_classification: bool}
```

`GET /tracker`

```
{login_status: number, nick_name: text, user_id: number-as-text, wearing_perfume_id: number, wearing_perfume_date: number, p_id: number, p_name: text, p_name_url: text, pic_name: text, p_gender: text, b_id: number, b_name: text, count: number, url_pic: text, perfumes: {lastworn: []}}
```

`GET /tracker_overview?offset=0&limit=20`

```
{login_status: number, has_more: bool, total_count: number, offset: number, limit: number, items: []}
```

`GET /tracker_calendar`

```
{login_status: number, year: number, month: number, today: text, is_current_month: bool, today_day: number, days_in_month: number, leading_blanks: number, week: {start: number, weekend: [number, …]}, min_year: number, max_year: number, totals: {wears: number, unique: number}, items: []}
```

`GET /tracker_stats`

```
{empty: bool}
```

`GET /notes_list?page=1&per_page=20`

```
{login_status: number, nick_name: text, user_id: number-as-text, notes: [], total_notes: number-as-text, page: number, per_page: number, total_pages: number}
```

`GET /rating?p_id=9438`

```
{login_status: number, nick_name: text, user_id: number-as-text, ratings: []}
```

`GET /classification?p_id=9438`

```
{status: text, locked: number, groups: [{c: number, name: text, options: [{…}, …]}, …]}
```

`GET /postbox?from=0`

```
{status: text, items: [], has_more: number, unread: number}
```

`GET /postbox/unread`

```
{status: text, unread: number}
```

`GET /sotd`

```
{wearing: bool}
```

`GET /feed?page=1&per_page=20`

```
{items: [], page: number, has_more: bool, wearing_today: [{author: {…}, perfume: {…}}, …]}
```

`GET /timezones`

```
{timezones: [{id: number, pretty_name: text, offset: text, time_zone: text}, …]}
```

`GET /compare/9438/1843`

```
{status: text, url_share: text, perfumes: [{con_ext_name: text, con_ext_concentration: text, id: number, name: text, name_url: text, brand: {…}, url_image_240: text, url_image_480: text, gender: text, release_year: number, perfumers: [{…}, …], rating: {…}, ratings: {…}, rank_gender: number, community: {…}, activity: {…}, classification: {…}, notes: {…}}, …], similarity: {score: number, band: text, notes: number, classifications: number, profile: number, votes: null}, overlap: {shared: {top: [{…}, …], middle: [{…}, …], base: [], notes: []}, unique: [{…}, …], cross: [], families: [{…}, …]}, popular: [{a: {…}, b: {…}}, …]}
```

`GET /compare_suggestions/9438`

```
{status: text, perfumes: [{con_ext_name: text, con_ext_concentration: text, p_id: number, p_name: text, b_name: text, url_image: text}, …]}
```

`GET /photos/9438`

```
{total: number, photos: [{id: number, url: text, thumb: text, title: text, awards: number, replies: number, user: {…}, perfume: {…}}, …], has_more: bool}
```

`GET /brand_details?b_id=253&page=1&per_page=20&g_f=1&g_m=1&g_u=1`

```
{b: {id: number-as-text, manufacturer_id: number-as-text, country_id: number-as-text, name: text, names_previous: null, names_alternative_language: null, name_url: text, name_sound_override: text, name_sound_update: number-as-text, name_sound_lang: text, name_sound_file: text, website: text, instagram: text, founding_year: number-as-text, nr_of_perfums: number-as-text, tags: text, avg_voting: number-as-text, avg_voting_month: number-as-text, total_votings: number-as-text, total_votings_month: number-as-text, in_wardrobes: number-as-text, pronunciation: text, info_text: text}, p: {matches: number, rows: [{…}, …]}}
```

`GET /perfumer_details?pf_id=509&page=1&per_page=20&g_f=1&g_m=1&g_u=1`

```
{pf: {id: number-as-text, country_id: number-as-text, name: text, name_url: text, website: text, role: text, role_en: text, role_pl: text, nr_of_perfums: number-as-text, in_wardrobes: number-as-text, avg_voting: number-as-text, avg_voting_dur: number-as-text, avg_voting_sillage: number-as-text, total_votings: number-as-text, total_votings_dur: number-as-text, total_votings_sillage: number-as-text, name_sound_file: null, name_sound_lang: null, name_sound_update: number-as-text, name_sound_override: null, pronunciation: text, info_text: null}, p: {matches: number, rows: [{…}, …]}}
```

`GET /fragrance_notes?feature=note&note_id=311&page=1&per_page=20`

```
{note: {id: number, name: text, nr: number, img: text, text: text, categories: [{…}, …]}, perfumes: [{id: number-as-text, p_name: text, p_name_url: text, concentration: text, gender: text, url_image_240: text, url_image_480: text, release_year: number-as-text, b_id: number-as-text, b_name: text, avg_voting: number-as-text, total_votings: number-as-text}, …], total_results: number, page: number, per_page: number, total_pages: number, gender: text, pyramid: text}
```

`GET /fragrance_notes?feature=category&cat_id=5&page=1&per_page=20`

```
{category: {id: number, name: text, name_url: text, nr_of_perfumes: number}, related_categories: [{id: number, name: text}, …], notes: [{id: number, name: text, nr: number, img: text}, …], perfumes: [{id: number-as-text, p_name: text, p_name_url: text, concentration: text, gender: text, url_image_240: text, url_image_480: text, release_year: number-as-text, b_id: number-as-text, b_name: text, avg_voting: number-as-text, total_votings: number-as-text}, …], total_results: number, page: number, per_page: number, total_pages: number, gender: text}
```

`GET /discover?feature=tops_flops&type=tops&gender=m&period=all`

```
{perfumes: [{id: number-as-text, p_name: text, p_name_url: text, concentration: text, gender: text, url_image_240: text, url_image_480: text, release_year: text, b_id: text, b_name: text, avg_voting: number-as-text, total_votings: number-as-text}, …], type: text, gender_filter: text}
```

`GET /discover/9438`

```
{perfumes: {popular: [{…}, …], related: [{…}, …], similar: [{…}, …]}}
```

`GET /users/<nick>`

```
{user: {id: number, nick_name: text, photo: text, cover: text, gender: text, country_id: number, country: text, country_code: text, reg_date: number, scores: number, followers: number-as-text, following: number-as-text, is_own: number, is_following: number, can_message: number, wearing: null, stats: {reviews: number, statements: number, photos: number}, collection: {have: number, had: number, want: number, tested: number, memo: number, filling: number, mini: number}, lists: []}}
```

`GET /users/<nick>/collection?type=have`

```
{type: text, perfumes: []}
```

`GET /users/<nick>/reviews?page=1&per_page=20`

```
{reviews: []}
```

`GET /users/<nick>/statements?page=1&per_page=20`

```
{statements: []}
```

`GET /users/<nick>/followers?page=1&per_page=30`

```
{type: text, users: [], page: number, has_more: bool}
```

`GET /users/<nick>/following?page=1&per_page=30`

```
{type: text, users: [], page: number, has_more: bool}
```

`GET /users/<nick>/photos`

```
{total: number, photos: [], has_more: bool}
```

`GET /comments/reviews/723108?page=1&per_page=20`

```
{item: {type: text, id: number, user_id: number}, total: number, page: number, has_more: bool, comments: []}
```

`GET /rating_details/9438/durability`

```
{status: text, type: text, total: number, avg: number, avg_text: text, distribution: {<number>: number, …}, following_count: number, sort: text, offset: number, has_more: bool, users: [{id: number, nick_name: text, photo: text, rating: number, date: text}, …]}
```

`GET /user_search?q=marco&limit=10`

```
{q: text, users: [{nick_name: text, photo: text}, …]}
```

`GET /postbox/search?q=test`

```
{status: text, q: text, results: []}
```

`GET /shelfcontrol?list=have`

```
{status: text, list: number, rows: [], brands: [], countries: [], notes: [], accords: [], perfumers: [], classes: [], groups: {occasion: text, season: text}, colors: [], columns: [], views: [], lists: [{id: number, type: text, name: text, icon: text, count: number}, …]}
```

`GET /wardrobe_profiler?type=have&section=overview`

```
{error: text}
```

`GET /similar?action=detail&type=similar&p_id=9438&s_id=55402`

```
{status: text, detail: {p_id: number, s_id: number-as-text, p_name: text, b_name: text, url_image: text, percentage: number, cnt_yes: number, cnt_no: number, cnt: number, user_vote: null, avg_voting: number-as-text, total_votings: number-as-text, accords: [{…}, …]}}
```

`GET /perfume_details/63363`

```
{b: {id: number-as-text, manufacturer_id: number-as-text, country_id: number-as-text, name: text, names_previous: null, names_alternative_language: null, name_url: text, name_sound_override: text, name_sound_update: number-as-text, name_sound_lang: text, name_sound_file: text, website: text, instagram: text, founding_year: number-as-text, nr_of_perfums: number-as-text, tags: text, avg_voting: number-as-text, avg_voting_month: number-as-text, total_votings: number-as-text, total_votings_month: number-as-text, in_wardrobes: number-as-text}, p: {id: number-as-text, brand_id: number-as-text, manufacturer_id: number-as-text, release_year: number-as-text, status: text, name: text, name_url: text, name_sound_override: text, name_sound_update: number-as-text, name_sound_file: text, name_sound_lang: text, gender: text, top_notes: [{…}, …], middle_notes: [{…}, …], base_notes: [{…}, …], notes: [{…}, …], perfumer1_id: number-as-text, perfumer2_id: number-as-text, perfumer3_id: number-as-text, perfumer4_id: number-as-text, bottle_designer: text, limited_edition: text, seasonal: text, bottle_edition: text, concentration_edition: text, ce_perfume_id: number-as-text, be_perfume_id: number-as-text, pic_name: text, total_votings: number-as-text, total_votings_dur: number-as-text, total_votings_sillage: number-as-text, total_votings_bottle: number-as-text, total_votings_pricing: number-as-text, avg_voting: number, avg_voting_dur: number, avg_voting_sillage: number, avg_voting_bottle: number, avg_voting_pricing: number, voting_distribution: text, voting_distribution_dur: text, voting_distribution_sillage: text, voting_distribution_bottle: text, voting_distribution_pricing: text, total_reviews: number, total_reviews_en: number-as-text}, i: [text, …], c_total: [number, …], c: [[{…}, …], …], c_poles: [{ct: number, n: text, c: text, cd: text}, …], wr: [], wr_lists: {collection_buttons: [{…}, …]}}
```

### With a few entries in the account

Recorded on the same day, after the owner of the test account added perfumes to two lists,
rated one, logged a wear and wrote a personal note, on the site itself.

`GET /wardrobe`

```
{login_status: number, nick_name: text, user_id: number-as-text, collection_buttons: [{type: text, icon: text, name: text, nr: number-as-text, in_wr: number}, …], url_web: text, note: {id: number, text: text, date: number}, has_rating: bool, has_classification: bool}
```

`GET /wardrobe?type=have`

```
{login_status: number, nick_name: text, user_id: number-as-text, wr: [{id: number-as-text, p_id: number-as-text, p_name: text, p_name_url: text, concentration: text, gender: text, url_pic: text, release_year: number-as-text, b_id: number-as-text, b_name: text, date_added: number-as-text, quantity: number-as-text, bottle_size: text, paid: number-as-text, paid_currency: text, worn: number, last_worn: number, note: text, ml_left: null}, …], list_details: {id: number, name: text, items: null, order: number, icon: text, hidden: text, notes: text}, url_web: text, order: text}
```

`GET /wardrobe?type=want`

```
{login_status: number, nick_name: text, user_id: number-as-text, wr: [{id: number-as-text, p_id: number-as-text, p_name: text, p_name_url: text, concentration: text, gender: text, url_pic: text, release_year: number-as-text, b_id: number-as-text, b_name: text, date_added: number-as-text, quantity: number-as-text, bottle_size: text, paid: number-as-text, paid_currency: text, worn: number, last_worn: number, note: text, ml_left: null}, …], list_details: {id: number, name: text, items: null, order: number, icon: text, hidden: text, notes: text}, url_web: text, order: text}
```

`GET /wardrobe?overview=1&per=20`

```
{login_status: number, nick_name: text, user_id: number-as-text, version: number-as-text, per: number, collections: [{type: text, icon: text, name: text, nr: number, order: text, wr: [{…}, …]}, …]}
```

`GET /shelfcontrol?list=have`

```
{status: text, list: number, rows: [{id: number, p: number, title: text, pic: text, brand: number, country: number, year: number, gender: text, conc: text, be: number, ratings: [null, …], community: number, votes: number, qty: number, bottle: text, ml: null, paid: number, cur: text, memo: text, added: number, last: number, worn: number, perfumers: [], accords: [number, …], seasons: [number, …], occasions: [number, …], notes: [number, …], custom: {…}, photos: [], statement: number, st_text: text, st_lang: text, review: number}, …], brands: {<number>: text, …}, countries: {<number>: [text, …], …}, notes: {<number>: text, …}, accords: {<number>: text, …}, perfumers: {<number>: text, …}, classes: {<number>: text, …}, groups: {occasion: text, season: text}, colors: {<number>: [text, …], …}, columns: [], views: [], lists: [{id: number, type: text, name: text, icon: text, count: number}, …]}
```

`GET /users/<own nick>`

```
{user: {id: number, nick_name: text, photo: text, cover: text, gender: text, country_id: number, country: text, country_code: text, reg_date: number, scores: number, followers: number-as-text, following: number-as-text, is_own: number, is_following: number, can_message: number, wearing: {id: number, p_name: text, p_name_url: text, concentration: text, gender: text, b_name: text, b_name_url: text, url_image_240: text, since: number, since_text: text}, stats: {reviews: number, statements: number, photos: number}, collection: {have: number, had: number, want: number, tested: number, memo: number, filling: number, mini: number}, lists: []}}
```

`GET /users/<own nick>/collection?type=have`

```
{type: text, perfumes: [{id: number, p_name: text, p_name_url: text, concentration: text, gender: text, release_year: number, b_id: number, b_name: text, b_name_url: text, url_image_240: text}, …]}
```

`GET /tracker`

```
{login_status: number, nick_name: text, user_id: number-as-text, wearing_perfume_id: number-as-text, wearing_perfume_date: number-as-text, p_id: number-as-text, p_name: text, p_name_url: text, pic_name: text, p_gender: text, b_id: number-as-text, b_name: text, count: number-as-text, url_pic: text, l_id: number-as-text, layered_with: null, perfumes: {lastworn: [{…}, …]}}
```

`GET /tracker_overview?offset=0&limit=20`

```
{login_status: number, has_more: bool, total_count: number, offset: number, limit: number, items: [{l_id: number, p_id: number, p_name: text, con_ext_name: text, p_name_url: text, b_name: text, b_name_url: text, url_pic: text, release_year: null, concentration: null, wear_date: number, wear_date_readable: text, statistics: {…}, layered_with_id: null, layered_with: null, comment: null}, …]}
```

`GET /tracker_calendar`

```
{login_status: number, year: number, month: number, today: text, is_current_month: bool, today_day: number, days_in_month: number, leading_blanks: number, week: {start: number, weekend: [number, …]}, min_year: number, max_year: number, totals: {wears: number, unique: number}, items: [{l_id: number, p_id: number, p_name: text, con_ext_name: text, p_name_url: text, b_name: text, b_name_url: text, url_pic: text, release_year: null, concentration: null, wear_date: number, wear_date_readable: text, statistics: {…}, layered_with_id: null, layered_with: null, comment: null, day: number, url_pic_small: text}, …]}
```

`GET /tracker_stats`

```
{hero: {total_wears: number-as-text, unique_fragrances: number-as-text, first_wear: number-as-text, layered_wears: number-as-text}, top_fragrances: [{cnt: number-as-text, last_date: number-as-text, p_name: text, p_name_url: text, p_id: number-as-text, pic_name: text, b_name: text, b_name_url: text, url_pic: text}, …], top_brands: [{cnt: number-as-text, b_name: text, b_name_url: text}, …], by_daytime: [{period: text, c: number}, …], by_weekday: [{d: number, c: number}, …], genders: [{gender: text, c: number-as-text}, …], scent_types: [{n: text, c: text, p: number-as-text}, …], insights: {wardrobe_total: number, wardrobe_worn: number, most_loyal: null, most_neglected: null}}
```

`GET /notes_list?page=1&per_page=20`

```
{login_status: number, nick_name: text, user_id: number-as-text, notes: [{id: number-as-text, p_id: number-as-text, text: text, date: text, perfume: {…}}, …], total_notes: number-as-text, page: number, per_page: number, total_pages: number}
```

`GET /sotd`

```
{wearing: bool, p_id: number, name: text, brand: text, image: text, since: number}
```

`GET /rating?p_id=120220`

```
{login_status: number, nick_name: text, user_id: number-as-text, ratings: {scent: number-as-text, durability: number-as-text, sillage: number-as-text, bottle: number-as-text, pricing: number-as-text}}
```

`GET /rating?p_id=229262`

```
{login_status: number, nick_name: text, user_id: number-as-text, ratings: []}
```

`GET /wardrobe?p_id=120220`

```
{login_status: number, nick_name: text, user_id: number-as-text, collection_buttons: [{type: text, icon: text, name: text, nr: number-as-text, in_wr: number}, …], url_web: text, note: {id: number-as-text, text: text, date: number-as-text}, has_rating: bool, has_classification: bool}
```

`GET /tracker_calendar?y=2026&m=11`

```
{login_status: number, year: number, month: number, today: text, is_current_month: bool, today_day: number, days_in_month: number, leading_blanks: number, week: {start: number, weekend: [number, …]}, min_year: number, max_year: number, totals: {wears: number, unique: number}, items: [{l_id: number, p_id: number, p_name: text, con_ext_name: text, p_name_url: text, b_name: text, b_name_url: text, url_pic: text, release_year: null, concentration: null, wear_date: number, wear_date_readable: text, statistics: {…}, layered_with_id: null, layered_with: null, comment: null, day: number, url_pic_small: text}, …]}
```
