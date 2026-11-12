# DISCOVERY — Jalons 0 et 0 bis : exploration de parfumo.com

11 novembre 2026 (jalon 0, partie publique), complété le 12 novembre 2026 (jalon 0 bis, partie connectée).
Rapport d'exploration prévu par les sections 4 et 4 bis du cahier des charges.
Point d'arrêt : les jalons 2 à 4 attendent la validation de ce rapport par le commanditaire.

## 0. Résumé

- **Le site public est entièrement lisible par simples requêtes HTTP**, sans JavaScript ni
  navigateur. Tout ce qui a été testé répond en 200 avec un User-Agent honnête.
- **La partie connectée aussi** (jalon 0 bis, 12 novembre 2026). La connexion par
  `POST /api/account/login` fonctionne avec le client du projet ; elle pose le cookie `pf_auth`,
  qui suffit à lui seul pour retrouver une session.
- **Lecture : presque pas de JSON**, sauf une exception de taille : ShelfControl embarque
  toute la collection en JSON dans sa page. **Écriture : les routes `/api/*` et
  `/action/**`** répondent en JSON, par une valeur nue ou par un corps vide.
- **Aucune fonction n'exige de navigateur** (méthode 3).
- **Collection, ports, notes, classification, messagerie, abonnements, réglages et profil
  sont confirmés** sur le compte de test : chaque écriture a été essayée une fois puis annulée.
- **Écritures publiques** (avis, statement, photo, forum, mur, blog, Souk, communautés) :
  routes et charges utiles relevées, rien envoyé.
- **Captcha** : aucun, ni à la connexion ni sur une écriture. Le seul captcha du site reste
  Cloudflare Turnstile, à l'inscription.
- **Restriction de compte récent** : la création d'offre au Souk est refusée au compte de test.
- **La règle « 200 avec corps vide = erreur » doit être nuancée** : neuf écritures répondent
  par un corps vide quand elles réussissent (section 3.9).

### Conditions de l'exploration

| Élément | Jalon 0 (public) | Jalon 0 bis (connecté) |
| --- | --- | --- |
| Requêtes vers parfumo.com | environ 210, en 13 minutes | environ 300, en 16 minutes |
| Débit | au moins 1,2 s entre deux requêtes | au moins 1,1 s, délai aléatoire |
| User-Agent | fixe : `parfumo-mcp-discovery/0.1 (personal-use exploration; honest fixed UA; curl)` | fixe : `parfumo-mcp/0.1 (personal-use MCP server; fixed honest UA)` |
| Outil | `curl` | exemple `explore` de `crates/parfumo-client`, transport de production |
| Compte | aucun | le compte de test seul ; trois connexions, toutes réussies |
| Contournements | aucun | aucun (pas de rotation d'IP, pas de captcha rencontré, pas d'imitation de navigateur) |

Convention de statut dans tout le document :

- **Testée** : requête réellement exécutée, résultat indiqué.
- **Vue, non testée** : route lue dans le HTML ou le JS du site, pas appelée.
- **Confirmée connecté** : exécutée sur le compte de test au jalon 0 bis.
- **Inexistante** : la fonction n'existe pas sur le site.

## 1. Fonctionnement général du site

| Sujet | Constat | Statut |
| --- | --- | --- |
| Rendu | PHP rendu côté serveur, jQuery 3.7.1, pas de SPA | Testé |
| Protection d'entrée | Cloudflare. Aucun défi reçu. Les pages embarquent le script passif `/cdn-cgi/challenge-platform/scripts/jsd/main.js` (détection JS de Cloudflare), sans effet sur des requêtes HTTP simples depuis cette machine | Testé |
| User-Agent honnête | Accepté (200) sur toutes les pages. Le comportement depuis l'IP du futur VPS n'est pas connu | Testé ici, à revérifier sur le VPS |
| Session | Cookie `PHPSESSID` posé dès la première page, même anonyme : `Max-Age=315360000` (10 ans), `Secure`, `HttpOnly`, `SameSite=Lax` | Testé |
| JSON servi en `text/html` | `/api/alive` et les `search_preview.php` renvoient du JSON avec `Content-Type: text/html`. Il faut détecter le JSON par le corps, pas par l'en-tête | Testé |
| Erreurs silencieuses | Un appel `/action/**` avec un jeton invalide, sans session ou en GET au lieu de POST renvoie **200 avec un corps vide**, pas un code d'erreur | Testé |
| 404 | Page HTML 404 standard (18 522 octets) pour toute route inconnue | Testé |
| Route protégée | 302 vers `/action/dologin` quand la route existe mais demande une connexion. Sert d'oracle : 302 = existe, 404 = n'existe pas | Testé |
| Site allemand | `www.parfumo.de` existe en parallèle (mêmes pages, URL en allemand). Hors périmètre, domaine configurable | Vu, non testé |
| Application mobile | `/app_v5/api/` existe (403 nginx). Non exploré : usurper l'application est interdit par le cahier des charges | Testé (403) |
| Temps réel | WebSocket `wss://parfumo.com/worker/status` pour le fil de statuts de la page d'accueil. Non nécessaire au serveur | Vu, non testé |

### Les deux jetons

| Jeton | Forme | Portée | Où le trouver | Statut |
| --- | --- | --- | --- | --- |
| `h` | 32 caractères hexadécimaux | Lié à un objet (parfum, avis, liste). **Identique d'une session à l'autre** et accepté sans cookie | Dans la page de l'objet : attribut `data-h` et scripts en ligne. Un même parfum en porte deux (un pour les listes, un pour le graphique de classification) | Testé |
| `csrf_key` | 16 caractères alphanumériques en anonyme, 32 hexadécimaux connecté | **Lié à la session** (`PHPSESSID`). Refusé sans le cookie correspondant | Scripts en ligne de chaque page, et `GET /api/alive` → `{"csrf_key":"…"}` | Testé |

Le JS du site lit aussi `<meta name="csrf-token">`, absent des pages anonymes et **présent sur
toutes les pages une fois connecté** (confirmé au jalon 0 bis : même valeur que `/api/alive`).
Sur un 403 `{"error":"Invalid CSRF token"}` (observé connecté, section 3.9), le JS rappelle
`/api/alive` puis rejoue la requête une fois.

Conséquence pour le futur client : lire la page de l'objet (pour `h`), garder la session
anonyme ou connectée (pour `csrf_key`), puis appeler les `/action/**` en POST
`application/x-www-form-urlencoded`. Une réponse vide doit être traitée comme une erreur pour une lecture ; pour certaines
écritures elle est le succès normal (section 3.9).

## 2. Carte des routes publiques

Toutes les pages ci-dessous sont en GET, renvoient une page HTML complète, et n'ont pas
besoin de JavaScript sauf mention contraire. Pagination : `?current_page=N` partout où elle existe.

### 2.1 Pages

| Fonction | URL | Paramètres | Statut |
| --- | --- | --- | --- |
| Accueil | `/` | — | Testée, 200 |
| Fiche parfum | `/Perfumes/{Marque}/{Parfum}` | — | Testée, 200. Contient : notes, perfumeurs, année, 5 notes moyennes (`scent`, `durability`, `sillage`, `bottle`, `pricing`), microdonnées schema.org (`aggregateRating`, `review`), 5 premiers avis et statements, identifiant numérique `p_id` |
| Avis isolé | `/Perfumes/{Marque}/{Parfum}/reviews/{id}` | — | Testée, 200 |
| Statement isolé | `/Perfumes/{Marque}/{Parfum}/statements/{id}` | — | Testée, 200 |
| Liste des marques | `/Brands` | `filter` | Testée, 200 |
| Fiche marque | `/Perfumes/{Marque}` | `current_page`, `o` (tri), `v=grid`, `g_f`/`g_m`/`g_u` (genre) | Testée, 200. Identifiant numérique `b_id` dans les liens |
| Maison mère | `/makers/{Nom}` ; index `/makers` | — | Fiche testée, 200 ; index vu, non testé |
| Liste des parfumeurs | `/Perfumers`, `/Perfumers/{lettre}` | `filter` | Testées, 200 |
| Fiche parfumeur | `/Perfumers/{Nom}` | `current_page`, `o`, genre | Testée, 200. Identifiant `pf_id` |
| Liste des notes | `/Fragrance_Notes` | — | Testée, 200 |
| Fiche note | `/Fragrance_Notes/{Note}` | `current_page`, `order` | Testée, 200 |
| Classement général / nouveautés | `/Perfumes` (« New Perfumes, Top100 ») | — | Testée, 200 |
| Top par genre | `/Perfumes/Tops/Men`, `/Women`, `/Unisex` | `current_page` | `Men` testée (pages 1 et 2), 200 ; `Women` et `Unisex` vues, non testées |
| Tendances | `/Perfumes/Trends` | — | Testée, 200 |
| Derniers ajouts | `/Recently_added` | `current_page`, genre | Testée, 200 |
| Par année | `/Release_Years`, `/Release_Years/{année}` | `current_page`, `o` | Testées, 200 |
| Dupes | `/Perfumes/Dupes` | recherche via `_get_dupes.php` (voir 2.2) | Testée, 200 |
| Découverte | `/Discover`, `/Discover/Tags`, `/Discover/Classification/Season` | — | Testées, 200. `/Discover/Classification/{Occasion,Style,Type}`, `/Discover/{Longevity,Sillage,Bottles,Colors,Random_Perfumes}` vues, non testées |
| Recherche avancée (formulaire) | `/search?show_search=1` | — | Testée, 200 |
| Recherche (résultats) | `/s_perfumes_x.php` | voir 2.3 | Testée, 200 |
| Derniers avis / statements | `/reviews`, `/reviews/top`, `/statements` | `current_page`, `sort` | Testées, 200. `/statements/top`, `/reviews/missing` vues, non testées |
| Photos | `/PerfumePictures` | `current_page`, `sort` | Testée, 200. `/PerfumePictures/top` vue, non testée |
| Forum : accueil | `/forums` (`/forum` redirige) | — | Testée, 200 |
| Forum : catégorie | `/forums/{catégorie}` | — | Testée, 200 (`perfume-discussions`) |
| Forum : fil | `/forums/topic/{slug}` puis `/forums/topic/{slug}/{page}` | 25 messages par page | Testées, 200 (pages 1 et 2) |
| Forum : message | `/forums/post/{id}` | — | Testée, 200 (affiche le fil) |
| Forum : recherche | `/s_forum_x.php` | `q` | Testée, 200 |
| Souk | `/Souks`, `/Souks/Sharings` | — | Testées, 200 |
| Souk : recherche | `/s_souk.php` | `filter`, `s_type`, `as`, `cs`, `img` (noms lus dans le formulaire ; `q` testé, 200, sens des champs non vérifié) | Testée, 200 |
| Blogs | `/blogs`, `/blogs/news` | `current_page`, `sort` | Testées, 200. `/blogs/{top,hot,search,interviews,stories,polls}` vues, non testées. `/New_Blogs` redirige vers `/blogs` |
| Billet de blog | `/Users/{pseudo}/Blog/Article/{slug}` | — | Testée, 200 |
| Communautés (groupes) | `/communities`, `/communities/{groupe}`, `/communities/{groupe}/post/{slug}` | — | Testées, 200. `/Community` redirige vers `/communities` |
| Membres actifs | `/Community/Top` | — | Testée, 200 |
| Activité | `/Community/Whats_happening_now` | — | Testée, 200 |
| Parfumo Research | `/fdb.php` (`/fdb` redirige) ; contributions d'un membre `/Users/{pseudo}/Research` | — | Testées, 200 |
| Parfumeries | `/Parfumerien` | — | Testée, 200 (page en allemand) |
| Plan du site | `/Sitemap` | — | Testée, 200 |
| Divers | `/videos`, `/podcast`, `/rules`, `/team`, `/support` | — | Vues, non testées |

### 2.2 Profils publics

| Fonction | URL | Statut |
| --- | --- | --- |
| Profil | `/Users/{pseudo}` | Testée, 200. Compteurs (listes, avis, blog, mur, abonnés, points), derniers avis |
| Collection (vue d'ensemble) | `/Users/{pseudo}/Collection` | Testée, 200. La page ne contient pas les parfums : elle les charge par `get_wardrobe.php` |
| Liste « J'ai » | `/Users/{pseudo}/Collection/I_have` (type interne `have`) | Testée, 200 |
| Liste « J'ai eu » | `/Users/{pseudo}/Collection/I_had` | Vue (lien du profil), non testée |
| Wish List | `/Users/{pseudo}/Collection/Wish_List` (type `want`) | Testée, 200 |
| Watch List | `/Users/{pseudo}/Collection/Watch_List` (type `memo`) | Testée, 200 (liste vide chez le membre testé) |
| Testés | `/Users/{pseudo}/Collection/Tested` (type `tested`) | Testée, 200 |
| Décants | `/Users/{pseudo}/Collection/Abfuellungen` (type `filling`) | Testée, 200 |
| Miniatures | `/Users/{pseudo}/Collection/Miniatures` | Testée, 200 mais **sans chargement de liste** chez le membre testé : la liste existe (identifiant 7, confirmé au jalon 0 bis) |
| Liste personnalisée | `/Users/{pseudo}/Collection/{id numérique}` | Testée, 200 |
| Avis longs | `/Users/{pseudo}/reviews` | Testée, 200 |
| Statements | `/Users/{pseudo}/statements` | Testée, 200 |
| Mur | `/Users/{pseudo}/Wall` | Testée, 200 |
| Blog | `/Users/{pseudo}/Blog` | Testée, 200 |
| Photos | `/Users/{pseudo}/PerfumePictures` | Testée, 200 |
| Souk du membre | `/Users/{pseudo}/Souk` | Testée, 200 |
| Notes données | `/Users/{pseudo}/Ratings` | Testée : **302 vers la connexion**. Existe, réservée aux connectés |
| Abonnés, amis, ports | `/Users/{pseudo}/Followers`, `/Friends`, `/Wearings`, `/Pictures` | Testées : 404. Ces URL devinées n'existent pas |

### 2.3 Recherche de parfums : `GET /s_perfumes_x.php`

Testée avec `filter=sauvage` : 200, page HTML, 151 résultats, pagination `current_page`.
Champs lus dans le formulaire `#s_form` (seuls `filter`, `v`, `o`, `current_page` et les genres
ont été exercés ; le sens des autres est déduit de leur libellé et **non testé**) :

| Champ | Rôle |
| --- | --- |
| `filter` | Texte libre |
| `o` | Tri : `n_asc`, `n_desc`, `p_id_desc` (nouveautés), `nr_desc`/`nr_asc` (popularité), `y_desc`/`y_asc` (année), `v_*` (senteur), `d_*` (tenue), `s_*` (sillage), `b_*` (flacon), `p_*` (rapport qualité-prix) |
| `v` | Vue (`grid`) |
| `g_m`, `g_f`, `g_u` | Genre : homme, femme, unisexe |
| `b_id`, `pf_id`, `m_id`, `c_id` | Identifiant de marque, parfumeur, maison mère, pays |
| `n_inc_id`, `n_exc_id`, `nc_inc_id`, `nc_exc_id` | Notes et catégories de notes à inclure ou exclure ; `n_t` (0/1) mode de combinaison |
| `y_t` (0/1/2), `y_0`, `y_1`, `y_2`, `y_3` | Année exacte, intervalle, décennie |
| `t`, `t_id`, `s_0`, `s_1`, `e_0`…`e_2`, `bc_0`…`bc_11`, `cl_1`…`cl_38`, `col_id` | Autres filtres (mots-clés, statut, concentration, couleurs, classification). Sens exact non vérifié |

Les identifiants numériques s'obtiennent par les autocomplétions `/action/s_x/label_filters_*.php`.

### 2.4 Appels internes publics (`/action/**`)

Tous en **POST** `application/x-www-form-urlencoded` (sauf mention), réponse en **fragment HTML**.
Aucun n'a besoin de JavaScript : il suffit de rejouer la requête avec les bons paramètres.

| Route | Paramètres | Jeton | Réponse | Statut |
| --- | --- | --- | --- | --- |
| `/action/perfume/get_reviews.php` | `p_id`, `f` (décalage), `n` (nombre), `t` (total), `o` (tri : `order_relevance_desc`, `order_date_desc`, `order_date_asc`, `order_scent_desc`, `order_scent_asc`, `order_replies_desc`), `full=Nein`, `append` | `h` | Avis longs paginés | Testée, 200. Fonctionne sans cookie |
| `/action/perfume/get_statements.php` | `p_id`, `f`, `n`, `t`, `o`, `append` | `h` | Statements paginés | Testée, 200 |
| `/action/perfume/get_reviews_text.php` | `id` (avis) | `h` | Texte complet d'un avis | Vue, non testée |
| `/action/perfume/get_similar.php` | `p_id`, `similar_type` (`similar`, `layer`, `tags`), `type` (`top`, `all`) | `h` | Parfums similaires ou étiquettes | Testée, 200 |
| `/action/perfume/get_inspiration.php` | `p_id`, `type` (`similar`, `layer`, `tags`), `similar_uservote_s_id`, `layer_uservote_s_id` | `h` | Bloc « similaires / à superposer / associations » | Testée, 200 |
| `/action/perfume/similar_sneak_peek.php` | `type`, `p_id`, `s_id` | `csrf_key` | Aperçu d'un parfum similaire | Testée, 200 |
| `/action/perfume/get_ratings_details.php` | `type` (`scent`, `durability`, `sillage`, `bottle`, `pricing`), `p_id`, `sort`, `offset`, `rows=1` pour la suite | `h` + `csrf_key` | Distribution des notes et votants | Testée, 200. Corps vide sans `csrf_key` |
| `/action/perfume/get_classification_chart.php` | `type` (`pie`, `radar`), `p` | `h` (second hash) + `csrf_key` | Classification en SVG ; les pourcentages sont lisibles dans `aria-label` | Testée, 200 |
| `/action/perfume/notes_popup.php` | `p_id`, `note_id`, `locked=No` | `csrf_key` | Détail d'une note | Testée, 200. Corps vide sans cookie de session |
| `/action/perfume/get_concentrations.php` | `p_id`, `p_ce` | `h` | Autres concentrations du parfum | Testée, 200 |
| `/action/perfume/get_imageries.php` | `p_id`, `f`, `n`, `t`, `o` | `h` | Photos des membres | Testée, 200 |
| `/action/perfume/_get_dupes.php` | **GET** `q` | — | Dupes correspondant à la recherche | Testée, 200 |
| `/action/perfume/get_review_translation.php`, `get_statement_translation.php` | `id`, `o` | `h` + `csrf_key` | JSON (texte original ou traduit) d'après le JS | Vues, non testées |
| `/action/perfume/smell_it.php`, `perfume_statement_panel.php` | `p_id` / `perfume_id`, `statement_id` | `csrf_key` | Panneaux (le second est le formulaire de statement) | Vues, non testées |
| `/action/comments/_get_comments.php` | `type` (`reviews`, `statements`, `blogs`, `wr_1`, `wr_3`…), `item_id`, `p1`, `p2`, `np`, `l` | `h` | Commentaires d'un avis, statement, billet ou liste | Testée, 200 (type `reviews`) |
| `/action/comments/_get_comment_translation.php` | `id`, `o` | `h` + `csrf_key` | JSON d'après le JS | Vue, non testée |
| `/action/wardrobe/get_wardrobe.php` | `id` (identifiant du membre), `nick`, `type` (`have`, `want`, `memo`, `tested`, `filling`, ou identifiant de liste), `type_url`, `gender`, `order`, `total_no_filter` | `csrf_key` | Contenu d'une liste : une vignette par flacon avec `data-id` (parfum) et `data-wr-id` (entrée) | Testée, 200, **sans être connecté** |
| `/action/wardrobe/wardrobe_sneak_peek.php` | `id` (= `data-wr-id`) | `csrf_key` | Détail d'une entrée : nom, marque, lien de la fiche | Testée, 200 |
| `/action/wardrobe/compare_panel.php` | `owner_id`, `list_id`, `group`, `offset` | `csrf_key` | Comparaison de collections | Vue, non testée |
| `/action/livesearch/livesearch.php` | `q`, `o` (`popular`, `latest`, `name`), `iwear` | aucun | Autocomplétion de parfums | Testée, 200 |
| `/action/livesearch/livesearch_suggestions.php` | **GET** | — | Recherches récentes | Testée, 200 |
| `/action/livesearch/livesearch_brand.php`, `_perfumer.php`, `_note.php` | `q` | `csrf_key` envoyé | Autocomplétion marques, parfumeurs, notes | Testées, 200 |
| `/action/s_x/label_filters_notes.php` | `q`, `input` | `csrf_key` envoyé | Notes avec identifiant (`data-n_inc_id`) | Testée, 200 |
| `/action/s_x/label_filters_{brands,perfumers,manufacturers,countries,notes_category,tags,users}.php` | `q`, `input` | idem | Identifiants pour les filtres de recherche | Vues, non testées |
| `/action/s_perfumes_x/search_preview.php` | champs du formulaire de recherche | `csrf_key` | **JSON** `{"matches":151,"matches_text":"Perfumes"}` | Testée, 200 |
| `/action/forum/search_preview.php` | `q` | `csrf_key` | **JSON** `{"matches":…}` | Testée, 200 |
| `/action/s_blogs/search_preview.php`, `/action/s_perfumes_x/share_link.php` | — | — | — | Vues, non testées |
| `/action/perfume/brands_countries_panel.php`, `perfumers_countries_panel.php`, `/action/perfumers/_get_perfumers_lists_sidebar.php`, `/action/comparison/compare_picker_panel.php`, `/action/groups/feed_fragment.php` | — | — | Panneaux annexes | Vues, non testées |

### 2.5 API JSON `/api/*`

| Route | Méthode | Constat | Statut |
| --- | --- | --- | --- |
| `/api/alive` | GET | `{"csrf_key":"…"}`, sans connexion | Testée, 200 |
| `/api/account/login` | POST | Connexion (voir section 4). En GET : 302 vers `/action/dologin?redirect=…` | Testée |
| `/api/forum/posts` | POST | `{"error":"Authentication required"}` | Testée, **401** |
| `/api/group/comments_render` | POST `post_id` | idem, avec ou sans `csrf_key` | Testée, **401** |
| `/api/gauge/get` | POST | 200, `application/json`, corps vide | Testée (sans paramètres utiles) |
| `/api/` | GET | 403 nginx (pas de listing) | Testée |
| `/api/forum/{reply,vote,pin,link_perfume,mark_read,mod_pic}` | POST | Écritures du forum | Vues dans `forum633.js`, non testées |
| `/api/subscription` | POST `type`, `sub_type`, `item_id`, `operation` (`add`/`remove`) | Abonnement à un fil | Vue, non testée |
| `/api/feedback` | POST `action` (`add`/`delete`), `type`, `item_id`, `feedback=useful` | « Utile » sur un message | Vue, non testée |
| `/api/group/{join,comment_create,comment_edit,comment_delete,comment_image_delete,post_pin_toggle}` | POST | Écritures des communautés | Vues, non testées |
| `/api/souk/{country_select,country_select_popup}` | POST | Filtre pays du Souk | Vues, non testées |
| `/api/account/delete_forum_pic` | POST `p` | Suppression d'une image de forum | Vue, non testée |
| `/api/filter` | GET `order`, `is_wr=1`, `wr_type` | Change le tri d'une liste de collection | Vue, non testée |
| `/api/darkmode` | GET | Bascule du mode sombre | Vue, non testée |
| `/api/fdb/*`, `/action_fdb/*` | POST | Parfumo Research (version 2) | Vues, non testées |

## 3. Routes du compte

Explorées au jalon 0 bis (12 novembre 2026) sur le compte de test, par requêtes HTTP, avec le
client de `crates/parfumo-client` (exemple `explore`, même User-Agent et même débit que la
production). Environ 300 requêtes en 16 minutes, une par seconde au plus. Aucun captcha, aucun blocage.

Statuts de cette section :

- **Confirmée connecté** : requête exécutée sur le compte de test, réponse relevée, essai annulé.
- **Vue, non testée** : route et charge utile lues dans le formulaire ou le script, rien envoyé.
- **Inexistante** : la fonction n'existe pas sur le site.

### 3.1 Règles communes

| Sujet | Constat |
| --- | --- |
| Deux familles de routes | `/api/{domaine}/{action}` (les scripts l'appellent `postAction`) et `/action/**.php`. Toutes en POST `application/x-www-form-urlencoded`, sauf mention |
| Jeton | Un seul : `csrf_key`, ajouté au corps de chaque POST. Connecté, il fait **32 caractères hexadécimaux** (16 alphanumériques en anonyme) et il est identique dans `<meta name="csrf-token">`, dans les scripts en ligne et dans `GET /api/alive` |
| Type de contenu | Presque toujours `text/html`, même quand le corps est du JSON. Seules les routes `/action/shelfcontrol/*` répondent `application/json` |
| Forme d'un succès | Variable d'une route à l'autre : JSON (`{"status":"ok"}`, `{"ok":true}`), valeur nue (`1`, un identifiant, `Yes`/`No`), fragment HTML, redirection 302, ou **corps vide**. Voir 3.9 |
| Identifiants | Parfum : `p_id` numérique (page du parfum). Entrée de collection : `wr_id` (renvoyé par l'ajout, présent dans ShelfControl). Port : `l_id`. Membre : identifiant numérique, lisible dans `/Postbox/R/{pseudo}` (`data-user`) et dans le profil (`data-item_id`) |

### 3.2 Collection

Listes fixes et leur identifiant : 1 `have` (I have), 2 `had` (I had), 3 `want` (Wish List),
4 `memo` (Watch List), 5 `tested` (Tested), 6 `filling` (Decants), 7 `mini` (**Miniatures :
existe**). Une liste personnalisée porte un identifiant numérique à six chiffres.

| Fonction | Requête | Réponse relevée | Statut |
| --- | --- | --- | --- |
| Listes et effectifs | `GET /account/shelfcontrol` : attribut `data-lists` (JSON : `id`, `slug`, `name`, `hidden`, `count`, `url`) | 7 listes fixes + listes personnalisées | Confirmée connecté |
| Contenu d'une liste, tous les champs | `GET /account/shelfcontrol/{slug ou id}` : objet JSON complet dans un `<script>` de la page (voir section 5) | `{"rows":[…],"brands":…}` | Confirmée connecté |
| Export CSV | `GET /action/wardrobe/generate_wardrobe_csv.php?list_id={id}` | `text/csv`, fichier joint | Confirmée connecté |
| Export PDF | `GET /action/wardrobe/generate_wardrobe_pdf.php?list_id={id}` | — | Vue, non testée |
| Chercher un parfum à ajouter | `POST /action/livesearch/livesearch_add_wr.php` (`q`, `type`, `hidden`, `silent`) | fragment | Vue, non testée |
| Ajouter à une liste | `POST /api/wardrobe/wardrobe_add_remove` : `p`, `t` (liste), `action=true`, `hidden=No` | corps = `wr_id` de l'entrée créée. Déjà présent : **corps vide**. Parfum inconnu : `0` | Confirmée connecté |
| Retirer d'une liste | même route avec `action=false`, ou `POST /api/wardrobe/wardrobe_remove` : `action=rw_{id membre}_{p_id}_{liste}` | corps vide. **Retirer de la liste 1 (I have) déplace le parfum dans la liste 2 (I had)** ; il faut un second retrait pour le faire disparaître | Confirmée connecté (les deux routes) |
| Copier ou déplacer | `POST /api/wardrobe/wardrobe_copy_move` : `wr_id`, `list_id`, `new_list_id`, `p_id`, `copy_move` (`copy`, `move`) | `1` ; en cas de refus, JSON `{"error_title","error_message"}` (lu dans le script) | Confirmée connecté (`copy`) |
| Opérations groupées | `POST /action/shelfcontrol/bulk.php` : `op` (`move`, `copy`, `remove`, `copy_list`), `list`, `target`, `had` (0/1), `ids` (séparés par des virgules) | `{"done":[…],"lists":[…]}` | Confirmée connecté (`move`) ; autres `op` vues |
| Modifier un flacon | `POST /api/wardrobe/update_details` : `id` (`wr_id`), `quantity`, `bottle_size`, `paid`, `currency` | `{"status":"ok","date_added":"12\/11\/2026"}`. **Répond aussi `ok` (avec `date_added` vide) pour un `id` qui n'appartient pas au compte** | Confirmée connecté |
| Contenance restante | `POST /action/shelfcontrol/columns.php` : `op=ml`, `row`, `value` | `{"ok":true}` | Confirmée connecté |
| Note privée d'un flacon | `POST /api/memo/create_memo` : `type=wardrobe`, `id`, `text` ; suppression `POST /api/memo/delete_memo` : `type`, `id` | `{"status":"ok","text":…,"date":…}` ; suppression : corps vide | Confirmée connecté |
| Colonnes personnalisées | `columns.php` : `op=save` (`id=0`, `name`, `type` = `check`, `text`, `date`), `op=value` (`row`, `id`, `value`), `op=delete` (`id`) | `{"ok":true}` | Confirmée connecté (les trois) |
| Vues enregistrées | `POST /action/shelfcontrol/views.php` : `op=save` (`name`, `icon`, `list`, `config` JSON), `update`, `edit`, `delete` | `{"id":…,"views":[…]}` | Confirmée connecté (`save`, `delete`) |
| Créer une liste | `POST /api/wardrobe/add_list` puis `set_wardrobe_name_notes` (`id`, `name` ≤ 40, `notes` ≤ 120) et `set_wardrobe_icon` (`id`, `icon`) | `{"status":"ok","id":…}`, `{"name":…,"notes":…}`, URL de l'icône | Confirmée connecté |
| Renommer une liste | `POST /api/wardrobe/set_wardrobe_name_notes` | idem | Confirmée connecté |
| Masquer une liste | `POST /api/wardrobe/wardrobe_lock_toggle` : `id` | `Yes` puis `No` | Confirmée connecté |
| Supprimer une liste | `POST /api/wardrobe/del_cust_wr` : `id` | corps vide ; la liste et ses entrées disparaissent | Confirmée connecté |
| Ordre manuel | `POST /api/wardrobe/update_wardrobe_order` : `type` (liste), `order` (tableau JSON de `wr_id`) | corps vide | Confirmée connecté |
| Vider une liste | `POST /api/wardrobe/empty_wardrobe` : `list_id` | `1` d'après le script | Vue, non testée |
| Photos privées d'un flacon | `POST /action/shelfcontrol/photos.php` (multipart ; `op=delete`, `id`) | JSON | Vue, non testée |
| Tri d'une liste | `GET /api/filter` (`order`, `is_wr=1`, `wr_type`) | — | Vue, non testée |
| Profil de collection, suggestions | `/action/wardrobe/profiler_*.php`, `/api/wardrobe/ignore_suggestion` | fragments | Vues, non testées |

### 3.3 Ports

| Fonction | Requête | Réponse relevée | Statut |
| --- | --- | --- | --- |
| Enregistrer un port (maintenant) | `POST /api/assistant/i_wear` : `p` = `p_id` | fragment HTML. Crée une entrée datée (`l_id`). **Le parfum porté s'affiche sur le profil public** (vérifié en anonyme). **Si le parfum n'est dans aucune liste, il est ajouté à la liste 5 (Tested)** | Confirmée connecté |
| Effacer le « parfum porté » affiché | même route, `p=clear` | fragment ; l'entrée de l'historique reste | Confirmée connecté |
| Enregistrer un port à une date passée | `POST /api/assistant/calendar_log` : `p_id`, `wear_date` | — | Vue, non testée |
| Historique | `GET /assistant/tracker` (lignes `#tablerow{l_id}` avec date), `/assistant/tracker_calendar`, `/assistant/recently_worn`, `/assistant/often_worn`, `/assistant/rarely_worn`, `/assistant/tracker_stats` ; mois : `POST /api/assistant/calendar_month` (`y`, `m`) | pages et fragment | Confirmée connecté (`tracker`, `tracker_calendar`, `recently_worn`, `calendar_month`) |
| Détail d'un port | `POST /api/assistant/tracker_details` : `l_id` | fragment (nombre de ports, premier port, présence en collection) | Confirmée connecté |
| Changer la date | `POST /api/account/update_iwear_date` : `id`, `new_date` (`AAAA-MM-JJTHH:MM`) | `1` | Confirmée connecté |
| Commentaire d'un port | `POST /api/account/update_iwear_comment` : `id`, `comment` (≤ 255) | `1` | Confirmée connecté |
| Superposition (layering) | `POST /api/account/update_iwear_layering` : `id`, `layered_with_id` (ou `clear`) | — | Vue, non testée |
| Supprimer un port | `POST /api/account/delete_iwear` : `id` | corps vide | Confirmée connecté |

Le site garde donc un historique daté : `parfumo_list_wears` et `parfumo_delete_wear` sont réalisables.
ShelfControl donne aussi, par flacon, `worn` (nombre de ports) et `last` (dernier port).

### 3.4 Notes et classification

| Fonction | Requête | Réponse relevée | Statut |
| --- | --- | --- | --- |
| Noter | `POST /api/perfume/add_vote` : `type` (`scent`, `durability`, `sillage`, `bottle`, `pricing`), `p_id`, `v` (0 à 100 ; le site affiche `v`/10), `silent=1` facultatif | `{"status":"ok"}` | Confirmée connecté (`scent`, `v=75`, puis retiré) |
| Retirer une note | `POST /api/perfume/remove_vote` : `type`, `p_id` | `{"status":"ok"}` | Confirmée connecté |
| Lire ses notes sur un parfum | `POST /action/perfume/perfume_rate_panel.php` : `perfume_id` (attributs `data-pstars-value`, en étoiles sur 5) | fragment | Confirmée connecté |
| Voter une classification | `POST /api/perfume/toggle_classification_vote` : `p`, `c` (groupe), `ct` (valeur), `action` (`add`, `remove`) ; les couples `c`/`ct` se lisent dans `/action/perfume/perfume_classify_panel.php` | `1` à l'ajout, `0` au retrait | Confirmée connecté |
| Notes données | `GET /Users/{pseudo}/Ratings` et `/Ratings/{scent,durability,sillage,bottle,pricing,similar,layer,tags}`, tri `?order=` | page | Confirmée connecté |
| Associations (étiquettes) | `POST /api/perfume/update_tag` : `o` (`add`, `remove`), `p`, `t` | — | Vue, non testée |
| Votes de similarité | `POST /action/perfume/my_similar_vote.php` ; `/api/perfume/update_similar_vote`, `remove_similar_vote` | — | Vues, non testées |

### 3.5 Avis, statements, photos, commentaires

Rien n'a été envoyé : ces contenus sont publics, soumis aux règles de rédaction du site et, pour
les photos, à une vérification par l'équipe.

| Fonction | Requête | Statut |
| --- | --- | --- |
| Formulaire de statement | `POST /action/perfume/perfume_statement_panel.php` : `perfume_id`, `statement_id` (0 = nouveau) | Confirmée connecté (lecture du formulaire) |
| Publier un statement | `POST /api/statement/create_statement` : `id` (parfum), `text` (≤ 140). Réponse JSON, `{"status":0,"msg":…}` en cas de refus | Vue, non testée |
| Supprimer un statement | `POST /api/statement/delete_statement` : `id`, `p`, `h` (lus sur le bouton du formulaire) | Vue, non testée |
| Formulaire d'avis | `POST /action/perfume/perfume_review_panel.php` : `perfume_id`, `review_id` | Confirmée connecté (lecture du formulaire) |
| Publier un avis | `POST /api/review/create_review` : `id`, `title` (≤ 200), `text` (≤ 7 500), `yt`, `paste_html` | Vue, non testée |
| Supprimer un avis | `POST /api/review/delete_review` : `id`, `p`, `h` | Vue, non testée |
| Relire son avis ou son statement | `POST /api/review/get_user_review` (`p`, `lang`), `/api/statement/get_user_statement` | Vues, non testées |
| Brouillons | `POST /api/core/autosave_save` (`field`, `content`), `/api/core/autosave_get` | Vues, non testées |
| Envoyer une photo | `POST /api/perfume/create_imagery`, multipart : `perfume_id`, `image_file`. Trois photos par parfum, consentement à cocher, contrôle par l'équipe et détection d'image générée | Vue, non testée |
| Commenter (avis, statement, billet, mur, liste) | `POST /action/comments/_post_comment.php` : `a` (type), `id`, `comment_id`, `text`, `p1`, `p2`, `redirect`, `l`, `h` ; réponse JSON `{"type","val","redirect"}` | Vue, non testée |
| Note personnelle sur un parfum (carnet) | `POST /api/memo/create_memo` : `type=perfume`, `id`, `text` (≤ 5 000) | Route confirmée avec `type=wardrobe` |

### 3.6 Social

| Fonction | Requête | Réponse relevée | Statut |
| --- | --- | --- | --- |
| **Messagerie : elle existe**, sous le nom Postbox | `GET /Postbox` (onglet non lus : `/Postbox/N`) | page | Confirmée connecté |
| Liste des conversations | `GET /action/postbox/_get_postbox_conv.php?n=` ; suite : `_get_postbox_conv_more.php?n=&f=25` | fragment | Confirmée connecté |
| Recherche | `GET /postbox_search.php?do_search=1&q=` | — | Vue, non testée |
| Conversation | `GET /Postbox/R/{pseudo}` : donne l'identifiant du membre (`data-user`) | page | Confirmée connecté |
| Messages d'une conversation | `GET /action/postbox/_get_postbox.php?r={id membre}&before=&before_date=&l=10` (ou `after=`) | lignes `<tr data-id data-time>` | Confirmée connecté |
| Envoyer | `POST /action/postbox/postbox_send.php` : `user_id`, `msg`, `temp_images` (0/1), `attachments[i]` | `1` ; sinon le corps est le texte de l'erreur | Confirmée connecté (un message au destinataire autorisé) |
| Supprimer un message | `POST /api/account/delete_pm` : `id` | corps vide ; le message disparaît de la boîte de l'expéditeur | Confirmée connecté. Effet chez le destinataire : non vérifiable depuis ce compte |
| Supprimer une conversation, bloquer | `POST /api/account/delete_conv` (`u`), `block_user`, `unblock_user` (`u`) | — | Vues, non testées |
| Pièce jointe | `POST /action/postbox/postbox_upload_attachment.php` (`name`, `size`, `type`, `data` en base64) | JSON `{"filename"}` | Vue, non testée |
| Notifications | `GET /api/notification/get_history` | fragment HTML (« No new notifications » sur ce compte) | Confirmée connecté (état vide seulement) |
| Marquer comme lu | `POST /api/notification/delete` (`id`), `/api/notification/delete_all` | — | Vues, non testées |
| Suivre un membre | `POST /api/subscription` : `operation` (`add`, `remove`), `item_id` (id membre), `type=users`, `sub_type=follow` | corps vide | Confirmée connecté (ajout puis retrait) |
| Liste des abonnements | `GET /account/follow/list` ; fil : `GET /account/follow?type={review,statement,photo,forum,group,blog,tier_list,video}` | pages | Confirmée connecté |
| S'abonner à un contenu | même route, `type=perfumes` ou `users`, `sub_type` = `reviews`, `statements`, `blogs`, `imageries_users`, `souks`, `souks_sharings` ; gestion : `GET /account/subscriptions` | — | Vue, non testée |
| Mur : écrire | `_post_comment.php` avec le type `id_cards` et l'identifiant du membre | — | Vue, non testée |
| Statut public | `POST /api/status/add_status` : `cf`, `s`, `target`, `vip`, `lang` | — | Vue, non testée |
| Forum : ouvrir un fil | formulaire : `POST /action/forum/compose_panel.php` (`forum_id`) ; envoi : `POST /api/forum/create_topic` : `forum_id`, `subject` (≤ 60), `message`, champs de sondage | JSON `{"success","error","redirect"}` | Formulaire confirmé connecté ; envoi vu, non testé |
| Forum : répondre, supprimer | `POST /api/forum/reply` (`topic_id`, `message`), `delete_post`, `delete_topic` | idem | Vues, non testées |
| Forum : divers | `/api/forum/{vote,pin,mark_read,link_perfume}`, `/api/feedback`, `/api/subscription` (`type=forum_{langue}`) | — | Vues, non testées |
| Blog : écrire | `GET /Users/{pseudo}/Blog/Write` ; `POST /action/blogs/post_blog.php` : `blog_write_post=1`, `blog_nick`, `blog_title` (≤ 200), `blog_text` (« Save & Preview ») | — | Formulaire confirmé connecté ; envoi vu, non testé |
| Souk : demande ou offre | formulaire : `POST /action/souk/souk_item_panel.php` (`perfume_id`, `t`, `souk_id`) ; envoi : `POST /api/souk/souk_adder` ou `souk_update` : `p`, `t`, `souk_id`, `souk_wunsch`, `zustand`, `tausch_gegen`, `ml_inhalt`, `unit`, `preisvorstellung`, `versand`, `notes` | — | Formulaire de demande confirmé connecté ; envoi vu, non testé. **Offre refusée à ce compte** (voir 3.8) |
| Souk : partage de flacon | `POST /action/souk/souk_sharing_panel.php` ; envoi `POST /api/souk/sharing_save` | — | Vue, non testée |
| Communautés | `POST /api/group/{join,leave,comment_create,comment_edit,comment_delete}` | — | Vues, non testées |

### 3.7 Profil, réglages, déconnexion

| Fonction | Requête | Réponse relevée | Statut |
| --- | --- | --- | --- |
| Pages de réglages | `GET /account/personal_details` (genre, pays, fuseau), `/account/notifications` (notifications et confidentialité, 14 cases), `/account/website` (langue, accès rapide, logo, priorité avis/statements, ordre du forum, 4 cases), `/account/subscriptions` | pages ; chaque champ porte `data-field` et son état | Confirmée connecté |
| Modifier un réglage | `POST /action/_aa_account.php` : `a` (nom du champ), `v` (`true`, `false` ou valeur de l'option) | **corps vide, que le champ existe ou non** : seul un nouveau GET de la page prouve l'effet | Confirmée connecté (`hide_note_pics`, activé puis rétabli) |
| Préférences olfactives (texte du profil) | `POST /fragrance_preferences.php` : `do_edit=1`, `i_like` (≤ 250), `i_dont_like` (≤ 250) | 302 vers `/Users/{pseudo}` | Confirmée connecté (rempli puis vidé) |
| Parfum signature | `POST /api/account/signature_scent` : `p` (`p_id` ou `clear`) ; recherche : `/action/livesearch/livesearch_sigscent.php` | fragment | Confirmée connecté (posé puis retiré) |
| Avatar, couverture, fond | `GET /account/profile_picture`, `/account/background` ; formulaires vers `_aa_account.php` ; `POST /api/account/delete_cover` | — | Vues, non testées |
| Ordre des blocs du profil | `POST /api/account/idcard_section_order` : `v` (tableau JSON) | — | Vue, non testée |
| Profil | `GET /Users/{pseudo}` (rang, pays, points, compteurs) | page | Confirmée connecté |
| Carnet | `GET /account/notebook`, `/account/notebook/{general,perfume,user,wardrobe}` | page | Confirmée connecté (page principale) |
| Top Picks, vidéos, album | `/Users/{pseudo}/top_picks` (`/api/tier_list/*`), `/videos`, `/Photo_Album` | — | Vues, non testées (hors catalogue) |
| Adresse postale | `GET /account/address_sharing` | — | Non visitée (données personnelles) |
| Déconnexion | `GET /api/account/logout` | 302 vers `/account/logged_out` ; voir section 4 | Confirmée connecté |
| Pseudo, e-mail, mot de passe, suppression | `/action/nickname_change`, `/action/email_change`, `/account/password_change`, `/account/delete` | — | Liens vus. **Jamais appelées** (interdits de la section 6) |
| Texte de présentation libre, parfums favoris | — | Aucun champ de ce nom. Ce qui existe : préférences olfactives, parfum signature, Top Picks | **Inexistante** sous cette forme |

### 3.8 Restrictions propres à un compte récent

Le compte de test a un jour d'ancienneté, 0 point et le rang « New user ».

| Zone | Constat |
| --- | --- |
| Souk | **Restriction avérée.** Le panneau affiche « Offer : Not available » avec un lien « Why can't I offer anything? » (`/rules/5`), et le formulaire d'offre revient presque vide. Seules les demandes de recherche sont proposées. La création d'offre n'est donc ni testée ni testable sur ce compte |
| Messagerie | Aucune restriction visible : le formulaire est présent et l'envoi a réussi. Un avertissement d'accueil s'affiche au premier passage (`POST /api/account/newbie_note`, non appelé) |
| Forum | Aucune restriction visible : les formulaires « nouveau fil » et « réponse » sont servis. Rien n'a été posté, une modération à l'envoi reste donc possible |
| Avis, statements, photos | Formulaires servis. Une modération ou un quota à l'envoi n'a pas pu être observé |
| Blog | Formulaire servi, envoi non testé |

Ces constats valent pour un compte neuf : un compte ancien peut voir d'autres options (offres du Souk, peut-être d'autres champs de profil).

### 3.9 Format des erreurs une fois connecté

| Cas | Réponse |
| --- | --- |
| `csrf_key` faux ou absent sur `/api/*` | **403**, corps `{"error":"Invalid CSRF token"}` (servi en `text/html`) |
| `csrf_key` faux sur `/action/**.php` | **200, corps vide** |
| Session absente ou expirée sur `/api/*` | **401**, corps `{"error":"Authentication required"}` |
| Session absente sur `/action/**.php` | 200 avec corps vide, ou un texte (`An error has occurred. Please try again later.` pour l'envoi d'un message) |
| Session absente sur une page ou sur l'export CSV | **302** vers `/action/dologin` |
| Paramètres manquants (corps réduit au jeton) | 200, corps vide |
| GET sur une route POST de `/api/*` | 200, corps vide |
| Route `/api/*` inconnue | 404, page HTML standard |
| Parfum inconnu à l'ajout | 200, corps `0` |
| Réglage inconnu | 200, corps vide (comme un succès) |
| Refus métier | JSON propre à la route : `{"status":0,"msg":…}` (avis, statement), `{"success":false,"error":…}` (forum), `{"error_title","error_message"}` (copie de liste), texte brut (messagerie). Lus dans les scripts, non déclenchés |
| Quota, limitation de débit | **Non observé** en 300 requêtes à une par seconde : ni 429, ni message de quota |
| Captcha | Aucun, ni à la connexion ni sur une écriture |

**Conséquence pour la règle « 200 avec corps vide = erreur ».** Elle reste vraie pour les lectures
(panneaux, fragments). Elle est fausse pour neuf écritures, dont le succès normal est un corps
vide : `wardrobe_remove`, retrait par `wardrobe_add_remove`, `del_cust_wr`, `update_wardrobe_order`,
`memo/delete_memo`, `account/delete_iwear`, `account/delete_pm`, `subscription`, `_aa_account.php`.
Pour celles-ci, la seule preuve est la relecture de l'état, déjà exigée par la section 8 du
cahier des charges.

## 4. Parcours de connexion

Trois connexions ont été faites au jalon 0 bis, toutes réussies (le maximum autorisé) : la
première pour l'exploration, la deuxième pour vérifier que le jeton de connexion est
réutilisable, la troisième pour retirer un reliquat d'essai (voir 9.2).

| Élément | Constat | Statut |
| --- | --- | --- |
| Formulaire | Présent dans l'en-tête de toutes les pages et sur `/action/dologin` (= `/login.php`) | Testé |
| Action | `POST https://www.parfumo.com/api/account/login`, `application/x-www-form-urlencoded` | Testé |
| Champs | `username` (pseudo ou e-mail), `password`, et trois champs cachés : `autologin=checked`, `login=Login`, `redirect=` (URL de retour) | Testé |
| CSRF | **Aucun jeton** dans le formulaire de connexion | Testé |
| Client non-navigateur | Accepté : User-Agent fixe du client, sans `Origin` ni `Referer` | **Confirmé** |
| Succès | **302 vers `https://www.parfumo.com/`**, corps vide | **Confirmé** (trois fois) |
| Cookies posés au succès | `pf_auth` (64 caractères hexadécimaux, `Max-Age=315360000` soit 10 ans, `Secure`, `HttpOnly`, `SameSite=Lax`), un nouveau `PHPSESSID` (mêmes attributs, 10 ans), et `m7er8padf9` (26 caractères, sans `HttpOnly`, rôle inconnu). La réponse efface aussi `user_id`, `last_login` et `user_id_key`, des noms hérités que le site ne pose plus | **Confirmé** |
| Cookie de connexion | **`pf_auth` suffit.** Présenté seul, il donne 200 sur une page protégée et le site émet un nouveau `PHPSESSID`. Réutilisable (vérifié deux fois de suite). Chaque connexion émet un `pf_auth` différent | **Confirmé** |
| `PHPSESSID` seul | Fonctionne tant que la session du serveur vit | **Confirmé** |
| `<meta name="csrf-token">` | **Présent sur toutes les pages une fois connecté** (absent en anonyme). Sa valeur est le `csrf_key` de la session : 32 caractères hexadécimaux, identique à `GET /api/alive` | **Confirmé** |
| Double authentification | Aucune : une seule requête suffit | **Confirmé** sur ce compte |
| Captcha | Aucun au formulaire, ni après un échec, ni après trois connexions réussies en un quart d'heure | Testé |
| Échec | 302 vers `/account/login_error`, corps vide, page « Invalid username / email or password ». La réponse efface `user_id`, `last_login`, `user_id_key` et repose un `PHPSESSID`. Même réponse que le compte existe ou non | Testé (jalon 0 : un échec ; premier essai du jalon 0 bis : deux échecs avec l'ancien mot de passe) |
| Déconnexion | `GET /api/account/logout`, **sans jeton** : 302 vers `/account/logged_out`, efface `pf_auth`, pose `last_login_user` (30 jours) et un nouveau `PHPSESSID` | **Confirmé** |
| Portée de la déconnexion | Elle **révoque le `pf_auth` côté serveur** : présenté ensuite seul, il renvoie vers la connexion. Une autre session ouverte par la même connexion a encore répondu 200 trente secondes plus tard, puis 302 quatre minutes plus tard (entre-temps une nouvelle connexion avait eu lieu : cause exacte non établie) | **Confirmé** pour la révocation ; délai non mesuré |
| Durée de session | Les deux cookies vivent 10 ans côté client. Côté serveur, la plus longue session observée a duré **12 minutes sans expirer** ; aucune expiration spontanée n'a été vue. La durée réelle reste inconnue | Partiel : à mesurer par l'usage |
| Lien de connexion par e-mail | `/account/otl_request` (« One-time access link via email ») | Vu, non testé |
| Mot de passe oublié | `/account/password_forgot` | Testé, 200 (page seulement) |
| Inscription | `/account/register` → `POST /account_create.php`, protégée par **Cloudflare Turnstile** | Page testée ; envoi non testé (volontairement) |

Conséquences pour la section 6 du cahier des charges :

- **Mode identifiants** : confirmé. Une seule requête POST, sans jeton préalable, sans navigateur.
- **Mode cookie** : il suffit d'importer **`pf_auth`**. `PHPSESSID` est facultatif, le site le
  recrée. Attention : si l'utilisateur clique sur « Logout » dans le navigateur d'où vient le
  cookie, ce `pf_auth` est révoqué et le serveur perd l'accès.
- **Renouvellement** : sur un 401 de `/api/*` ou un 302 vers `/action/dologin`, le serveur
  rejoue d'abord avec `pf_auth` seul (nouveau `PHPSESSID`), puis se reconnecte par
  identifiants si `pf_auth` est refusé.
- **Jeton** : relire `csrf_key` après chaque nouvelle session ; il change avec elle.
- **Le serveur ne doit jamais appeler `/api/account/logout`** pour un compte lié en mode
  cookie : cela couperait aussi le navigateur de l'utilisateur.
- **Captcha** : le seul captcha du site reste Turnstile, à l'inscription. S'il apparaît un
  jour à la connexion, la seule issue est `captcha_required` puis import de cookie.

## 5. ShelfControl et export de la collection (section 4.6)

Tout est confirmé connecté. Il existe **trois** façons de lire une collection, toutes par
simple requête, sans navigateur.

| Voie | Requête | Contenu | Verdict |
| --- | --- | --- | --- |
| **JSON de ShelfControl** | `GET /account/shelfcontrol/{slug ou id de liste}` | Un objet JSON complet dans un `<script>` de la page | **À retenir** : la source la plus riche |
| CSV du serveur | `GET /action/wardrobe/generate_wardrobe_csv.php?list_id={id}` | `text/csv; charset=utf-8`, BOM UTF-8, `Content-Disposition: attachment; filename=i-have-12-11-2026.csv` | Simple, mais 10 colonnes seulement |
| Bouton CSV de ShelfControl | aucune requête : le fichier est fabriqué dans le navigateur (`Blob`) à partir du JSON ci-dessus | Colonnes visibles du tableau | Non rejouable, et inutile : le JSON contient les mêmes données |

**JSON de ShelfControl.** La page porte la liste active dans `data-list`, toutes les listes
dans `data-lists`, les vues enregistrées dans `data-views`. L'objet a la forme
`{"rows":[…],"brands":{…},"countries":{…},"genders":{…},"notes":{…},"accords":{…},"perfumers":{…},"classes":{…},"groups":{…}}`.
Chaque ligne de `rows` contient :

| Champ | Sens |
| --- | --- |
| `id`, `p` | `wr_id` de l'entrée, `p_id` du parfum |
| `title`, `name`, `url`, `pic`, `conc`, `year`, `gender` | Identité du parfum (`name` contient du HTML) |
| `brand`, `country`, `perfumers`, `accords`, `notes`, `seasons`, `occasions` | Identifiants, résolus par les dictionnaires voisins |
| `ratings` | Les cinq notes du membre (senteur, tenue, sillage, flacon, prix), `null` si absente |
| `community`, `votes` | Note moyenne et nombre de votes |
| `qty`, `bottle`, `ml`, `paid`, `cur` | Quantité, contenance, contenance restante, prix payé, devise |
| `memo` | Note privée |
| `added`, `added_f` | Date d'ajout (horodatage Unix et texte) |
| `worn`, `last`, `last_f` | Nombre de ports, dernier port |
| `custom`, `photos` | Colonnes personnalisées, photos privées |
| `statement`, `st_text`, `st_lang`, `review` | Statement et avis du membre sur ce parfum |

**CSV du serveur.** Colonnes : `Perfume`, `Brand`, `Year`, `Gender`, `Quantity`, `Bottle Size`,
`Paid`, `Date of Entry`, `Notes`, `Notebook`. Séparateur virgule, champs entre guillemets.
Ni les notes du membre, ni les ports, ni les colonnes personnalisées n'y figurent.

**Écritures de ShelfControl** : voir 3.2 (cellules, colonnes personnalisées, vues, opérations
groupées). Elles ont toutes été essayées puis annulées.

**Repli public** : `get_wardrobe.php` + `wardrobe_sneak_peek.php` restent la seule voie pour
lire la collection d'un **autre** membre ; ShelfControl ne montre que la sienne.

Fixtures : `auth_shelfcontrol.html`, `auth_shelfcontrol_empty.html`, `auth_wardrobe_export.csv`.

## 6. Versions citées en section 1 et 5

| Sujet | Trouvé | Source |
| --- | --- | --- |
| `rmcp` | **3.9.0**, publiée le 5 novembre 2026 (3.8.0 le 26 octobre). Dépôt : `modelcontextprotocol/rust-sdk` | API crates.io : `https://crates.io/api/v1/crates/rmcp`, consultée le 11 novembre 2026 |
| Fonctions `rmcp` demandées | `server`, `macros`, `transport-streamable-http-server` et `auth` existent toutes dans 3.9.0 (ainsi que `transport-streamable-http-server-session`, `schemars`, `tower`) | Même réponse crates.io, champ `features` |
| Versions de protocole gérées par `rmcp` | Le README annonce la spécification `2026-07-28` avec compatibilité `2025-11-25` et antérieures | `README.md` de la branche `main` du dépôt (non vérifié sur le code de la version 3.9.0 elle-même) |
| Spécification MCP courante | **`2026-07-28`** : `https://modelcontextprotocol.io/specification/latest` redirige (307) vers `/specification/2026-07-28` | modelcontextprotocol.io, consultée le 11 novembre 2026 |
| Protocole sans état | Confirmé : suppression de la poignée de main `initialize` ; chaque requête porte sa version et les capacités du client dans `_meta` | `https://modelcontextprotocol.io/specification/2026-07-28/changelog.md` |
| En-têtes `Mcp-Method` et `Mcp-Name` | Confirmé : requis sur les requêtes POST Streamable HTTP (SEP-2243) | Même changelog |
| Enregistrement dynamique des clients | Confirmé : déclassé (« Deprecated ») au profit des Client ID Metadata Documents | Même changelog |

Les affirmations du cahier des charges sur ces points sont exactes à la date de l'exploration.

## 7. Méthode retenue par fonction

Méthodes : **1** = appel interne rejoué en HTTP, réponse JSON ou valeur simple ; **2** = requête
HTTP et analyse du HTML (page ou fragment renvoyé par `/action/**`) ; **3** = impossible sans
navigateur.

| Fonction | Méthode | Justification |
| --- | --- | --- |
| Recherche de parfums | 2 | `/s_perfumes_x.php` rend la liste en HTML. Le seul JSON est le compteur `search_preview.php` |
| Fiche parfum | 2 | Page HTML + fragments `get_classification_chart`, `get_ratings_details`, `get_concentrations` |
| Avis, statements d'un parfum | 2 | Fragments `get_reviews.php`, `get_statements.php`, paginés par `f`/`n` |
| Similaires et dupes | 2 | Fragments `get_similar.php`, `get_inspiration.php`, `_get_dupes.php` |
| Marque, parfumeur, note, classements | 2 | Pages HTML paginées |
| Lecture d'une page quelconque | 2 | GET et nettoyage du HTML |
| Profil public, mur, blog, avis d'un membre | 2 | Pages HTML |
| Forum, Souk, communautés : lecture | 2 | Pages HTML |
| Connexion, déconnexion | 1 | POST de formulaire ; résultat lu dans la redirection et le cookie `pf_auth` |
| **Lecture de sa collection** | **1** (dans une page) | JSON complet de ShelfControl, une requête par liste. CSV du serveur en complément |
| Lecture de la collection d'un autre membre | 2 | Fragments `get_wardrobe.php` et `wardrobe_sneak_peek.php` |
| **Écritures de collection** | **1** | `/api/wardrobe/*`, `/api/memo/*`, `/action/shelfcontrol/*` : JSON, valeur nue ou corps vide ; relecture par ShelfControl |
| **Ports** | **1** pour écrire, **2** pour lire | `/api/assistant/i_wear`, `/api/account/*_iwear*` ; historique dans la page `/assistant/tracker` |
| **Notes, classification** | **1** pour écrire, **2** pour lire | `/api/perfume/add_vote`, `remove_vote`, `toggle_classification_vote` ; lecture par le panneau de notation, ShelfControl ou `/Users/{pseudo}/Ratings` |
| Avis, statements : écrire, supprimer | 1 (vue, non testée) | `/api/review/*`, `/api/statement/*`, réponse JSON |
| Photo d'un parfum | 1, multipart (vue, non testée) | `/api/perfume/create_imagery` |
| **Messagerie** | **2** pour lire, **1** pour écrire | Fragments `/action/postbox/_get_postbox*.php` ; `postbox_send.php` répond `1` |
| **Notifications** | 2 | `/api/notification/get_history` renvoie un fragment HTML |
| **Suivre un membre** | **1** pour écrire, **2** pour lire | `/api/subscription` ; liste dans `/account/follow/list` |
| **Réglages** | **2** pour lire, **1** pour écrire | Pages `/account/{personal_details,notifications,website}` ; `_aa_account.php` (corps vide) |
| **Profil** | 2 | Formulaire `/fragrance_preferences.php` (302), fragment de `signature_scent` |
| Forum, communautés : écrire | 1 (vue, non testée) | `/api/forum/*`, `/api/group/*`, réponse JSON |
| Mur, commentaires | 1 (vue, non testée) | `_post_comment.php`, réponse JSON |
| Blog : écrire | 2 (vue, non testée) | Formulaire classique `/action/blogs/post_blog.php` |
| Souk : écrire | 1 (vue, non testée) | `/api/souk/souk_adder`, `souk_update`, `sharing_save` |
| **Méthode 3** | **aucune** | Aucune fonction du catalogue n'exige de navigateur. Le seul élément fabriqué par JavaScript, le bouton CSV de ShelfControl, a deux équivalents côté serveur |

## 8. Liste finale des outils

Statut de chaque outil après le jalon 0 bis :

- **Confirmé connecté** : toutes ses routes ont été exécutées sur le compte de test.
- **Prêt** : lecture publique, testée au jalon 0.
- **Vu, non testé** : routes et charges utiles lues, écriture publique non envoyée.
- **Partiel** : une partie confirmée, l'autre vue.
- **Retiré** : proposé au retrait, avec la raison.

### Base de données (10 outils, tous prêts)

Inchangé : `parfumo_search_perfumes`, `parfumo_get_perfume`, `parfumo_get_perfume_reviews`,
`parfumo_get_perfume_statements`, `parfumo_get_similar_perfumes`, `parfumo_get_brand`,
`parfumo_get_perfumer`, `parfumo_get_note`, `parfumo_get_rankings`, `parfumo_fetch_page`.
Développés au jalon 1.

### Collection (9 outils, tous confirmés)

| Outil | Statut | Routes et remarques |
| --- | --- | --- |
| `parfumo_list_collections` | Confirmé connecté | `data-lists` de ShelfControl. Les sept listes fixes existent, Miniatures comprise |
| `parfumo_get_collection` | Confirmé connecté | JSON de ShelfControl pour son compte ; `get_wardrobe.php` pour un autre membre |
| `parfumo_add_to_collection` | Confirmé connecté | `wardrobe_add_remove`. Déjà présent = corps vide : l'idempotence demandée par la section 8 est native |
| `parfumo_move_collection_item` | Confirmé connecté | `wardrobe_copy_move` ou `bulk.php`. Paramètre `copy` à prévoir, le site le propose |
| `parfumo_update_collection_item` | Confirmé connecté | `update_details`, `columns.php` (`ml`, `value`), `memo/create_memo`. Champs réels : quantité, contenance, contenance restante, prix et devise, note privée, colonnes personnalisées. **Pas de champ « date d'achat »** : la date d'ajout est fixée par le site |
| `parfumo_remove_from_collection` | Confirmé connecté | Retirer de « I have » déplace vers « I had ». L'outil doit le dire et offrir `also_from_had` |
| `parfumo_manage_custom_collection` | Confirmé connecté | `add_list`, `set_wardrobe_name_notes`, `set_wardrobe_icon`, `wardrobe_lock_toggle`, `del_cust_wr` |
| `parfumo_export_collection` | Confirmé connecté | JSON : ShelfControl. CSV : `generate_wardrobe_csv.php`, ou CSV reconstruit depuis le JSON pour avoir toutes les colonnes |
| `parfumo_collection_stats` | Confirmé connecté | Calculé depuis le JSON de ShelfControl, qui donne déjà marques, notes, prix et ports |

### Ports (3 outils, tous confirmés)

| Outil | Statut | Routes et remarques |
| --- | --- | --- |
| `parfumo_log_wear` | Confirmé connecté | `assistant/i_wear` ; date passée par `calendar_log` (vue) ou `update_iwear_date` (confirmée). **Type P** : le parfum porté s'affiche sur le profil public. Effet de bord : ajout à « Tested » si le parfum n'est dans aucune liste |
| `parfumo_list_wears` | Confirmé connecté | `/assistant/tracker`, `calendar_month` |
| `parfumo_delete_wear` | Confirmé connecté | `account/delete_iwear` |

### Notes et avis (8 outils au catalogue, 7 retenus)

| Outil | Statut | Routes et remarques |
| --- | --- | --- |
| `parfumo_rate_perfume` | Confirmé connecté | `perfume/add_vote`, `remove_vote`. Cinq échelles, valeur 0 à 100 |
| `parfumo_classify_perfume` | Confirmé connecté | `perfume/toggle_classification_vote` ; options lues dans le panneau |
| `parfumo_list_my_ratings` | Confirmé connecté | `/Users/{pseudo}/Ratings` |
| `parfumo_write_review` | Vu, non testé | `review/create_review` |
| `parfumo_write_statement` | Vu, non testé | `statement/create_statement` |
| `parfumo_delete_review_or_statement` | Vu, non testé | `review/delete_review`, `statement/delete_statement` |
| `parfumo_list_my_reviews` | Prêt | `/Users/{pseudo}/reviews` et `/statements` |
| `parfumo_upload_perfume_photo` | **Retiré** (proposition) | La route existe (`perfume/create_imagery`), mais chaque photo est vérifiée à la main par l'équipe, contrôlée contre les images générées, et limitée à trois par parfum. Un outil MCP devrait en plus recevoir un fichier binaire. Même logique que le report de Parfumo Research en version 2 |

### Social (13 outils du catalogue, 1 ajout proposé)

| Outil | Statut | Routes et remarques |
| --- | --- | --- |
| `parfumo_get_user` | Prêt | |
| `parfumo_list_messages` | Confirmé connecté | `_get_postbox_conv.php` |
| `parfumo_read_message_thread` | Confirmé connecté | `/Postbox/R/{pseudo}` puis `_get_postbox.php` |
| `parfumo_send_message` | Confirmé connecté | `postbox_send.php`. Suppression possible par `account/delete_pm` |
| `parfumo_get_notifications` | Partiel | `notification/get_history` confirmée, mais seul l'état vide a été vu : la forme d'une notification reste à capturer |
| `parfumo_manage_follow` | Confirmé connecté | `subscription`, `/account/follow/list` |
| `parfumo_wall` | Partiel | Lecture prête ; écriture vue, non testée |
| `parfumo_forum_browse`, `parfumo_forum_read_thread` | Prêts | |
| `parfumo_forum_post` | Vu, non testé | `forum/create_topic`, `forum/reply` ; suppression `delete_post`, `delete_topic` |
| `parfumo_blog` | Partiel | Lecture prête ; écriture vue, non testée |
| `parfumo_souk_browse` | Prêt | |
| `parfumo_souk_manage_offer` | Vu, non testé | **Création d'offre refusée au compte de test** (compte récent). Seule la demande de recherche est accessible |
| `parfumo_communities` (ajout proposé) | Partiel | Lecture prête ; `group/join`, `group/comment_create` vues, non testées |

### Profil et compte (5 outils)

| Outil | Statut | Routes et remarques |
| --- | --- | --- |
| `parfumo_get_my_profile` | Confirmé connecté | `/Users/{pseudo}` et `/account/personal_details` |
| `parfumo_update_profile` | Confirmé connecté, **périmètre réduit** | Ce qui existe : genre, pays, fuseau (`_aa_account.php`), préférences olfactives (`fragrance_preferences.php`), parfum signature (`account/signature_scent`). **Retirés du rôle** : texte de présentation et parfums favoris (inexistants sous cette forme), avatar (envoi de fichier) |
| `parfumo_get_settings` | Confirmé connecté | Pages `/account/notifications` et `/account/website` |
| `parfumo_update_settings` | Confirmé connecté | `_aa_account.php`, avec relecture obligatoire |
| `parfumo_session_status` | Prêt | État local + `GET /api/alive` |

### Bilan

- **48 outils retenus** : 48 au catalogue, plus `parfumo_communities`, moins `parfumo_upload_perfume_photo`. Sous le plafond de 50.
- **23 confirmés connecté**, **16 prêts** (lecture publique), **4 partiels**, **5 vus, non testés**.
- Aucune fonction n'exige de navigateur.
- Fonctions du catalogue qui n'existent pas sur le site : le texte de présentation libre et les
  « parfums favoris » du profil, et le champ « date d'achat » d'un flacon.
- Fonctions du site hors catalogue, à trancher : carnet (`/account/notebook`, notes privées sur
  un parfum ou un membre), Top Picks (`/api/tier_list/*`), statistiques de ports
  (`/assistant/tracker_stats`), conseiller (`/assistant/consultant`), statut public, comparaison
  de collections. Parfumo Research reste en version 2.

## 9. Bilan du jalon 0 bis et points restants

### 9.1 Liste de travail du jalon 0

| # | Point | Résultat |
| --- | --- | --- |
| 1 | Réponse d'une connexion réussie | **Confirmé** (section 4) |
| 2 | Cookies à importer, durée de session | Cookie : **confirmé** (`pf_auth`). Durée : 12 minutes observées, durée réelle inconnue |
| 3 | Déconnexion | **Confirmé** : `GET /api/account/logout` |
| 4 | ShelfControl : données, export, écritures | **Confirmé** (section 5 et 3.2) |
| 5 | Collection : écritures, Miniatures, champs privés | **Confirmé** (3.2) |
| 6 | Ports | **Confirmé** (3.3). Un port est visible publiquement |
| 7 | Notes, classification, `/Ratings` | **Confirmé** (3.4) |
| 8 | Avis, statements, photos | Routes et charges utiles **vues, non testées** (3.5) |
| 9 | Messagerie | **Confirmé** : elle existe (Postbox) ; envoi, lecture, suppression (3.6) |
| 10 | Notifications | Route confirmée, **état vide seulement** |
| 11 | Réglages et profil | **Confirmé** (3.7) |
| 12 | Suivre un membre | **Confirmé** (3.6) |
| 13 | Mur, blog, Souk, forum, communautés : écritures | **Vues, non testées** (3.6) |
| 14 | Format des erreurs connecté | **Confirmé** sauf quota (3.9) |
| 15 | Turnstile à la connexion ou sur une écriture | **Non apparu** |
| 16 | Cloudflare depuis l'IP du VPS | Inchangé : à tester sur le VPS |

### 9.2 Ce qui a été écrit sur le compte de test, et son état final

Chaque essai a été annulé. État final vérifié avant la dernière déconnexion : sept listes à
zéro, aucun port, aucune note, aucun abonnement, boîte de messages vide, réglages et
préférences rétablis, aucune liste, colonne ou vue personnalisée.

Quatre points à connaître :

- **Un message privé** a été envoyé au destinataire autorisé, puis supprimé côté expéditeur.
  Il peut rester visible dans la boîte du destinataire.
- **Le destinataire autorisé a été suivi puis désabonné** en quelques secondes. Il a pu
  recevoir une notification.
- **Deux contenus ont été publics un court instant** : une note de 7,5 en senteur sur un
  parfum très voté (envoyée avec `silent=1`, retirée aussitôt) et un vote de classification,
  ainsi qu'un port affiché environ une minute sur le profil.
- **Un essai a visé un identifiant qui n'était pas celui du compte** : `update_details` avec
  `id=1`, pour relever le format d'erreur. Le site a répondu `ok` avec une date vide, ce qui
  indique qu'aucune ligne du compte n'a été trouvée, mais l'absence d'effet sur une entrée
  d'un autre membre n'a pas pu être vérifiée. Cet essai n'a pas été répété.

Un reliquat a exigé la troisième connexion : le port de test avait ajouté le parfum à la liste
« Tested », ce qui n'a été vu qu'au contrôle final. Il a été retiré.

### 9.3 Ce qui reste incertain

1. Durée réelle d'une session côté serveur, et délai exact entre une déconnexion et la fin
   des autres sessions de la même connexion.
2. Forme d'une notification non vide.
3. Effet de `account/delete_pm` chez le destinataire.
4. Toutes les écritures publiques : avis, statement, photo, commentaire, mur, statut, forum,
   blog, Souk, communautés. Les charges utiles sont déduites des formulaires et des scripts ;
   modération, quotas et messages de refus sont inconnus.
5. Création d'offre au Souk : inaccessible à un compte récent.
6. Quotas et limitation de débit du site : jamais atteints.
7. Comportement de Cloudflare depuis l'IP du VPS.
8. Rôle du cookie `m7er8padf9`.

### 9.4 Fixtures

48 fixtures `auth_*` dans `crates/parfumo-client/tests/fixtures/` : pages (`auth_*.html`),
fragments (`auth_frag_*.html`), réponses d'API (`auth_api_*.json`), export
(`auth_wardrobe_export.csv`). Expurgées : pseudos remplacés par `testuser` et `otheruser`,
identifiants de membres remplacés, tout jeton de 32 caractères hexadécimaux ou plus remplacé
par `0123456789abcdef…`, adresse e-mail et pays retirés. Trois tests les couvrent dans
`tests/parse_fixtures.rs`, dont un qui échoue si une fixture contient un jeton réel.

### 9.5 Outil d'exploration

`cargo run -p parfumo-client --features explore --example explore -- <login | csrf | req …>`.
Une requête par appel, par le transport de production, état et cookies hors du dépôt
(`/tmp/parfumo-explore`). Le script de capture HAR du jalon 0, devenu inutile, a été retiré du dépôt.

## 10. Mesures du jalon 2 (12 novembre 2026)

Relevées en développant les outils de collection et de ports, sur le compte de test, à
travers le serveur (une connexion, une centaine de requêtes, une par seconde au plus). Aucun
captcha, aucun blocage, aucun 429. Tout a été annulé : sept listes à zéro en fin d'essai.

| Sujet | Constat | Statut |
| --- | --- | --- |
| `GET /account/shelfcontrol/{id}` | Fonctionne pour une liste personnalisée désignée par son identifiant numérique | Confirmé |
| Entrée neuve | `qty` vaut 1 et `cur` vaut déjà `EUR` (devise par défaut du compte), sans prix | Confirmé |
| `wardrobe_copy_move`, `move` | Le flacon garde ses champs privés (contenance, prix, note) | Confirmé |
| `wardrobe_copy_move`, `copy` | La copie est une entrée neuve, **sans** les champs privés | Confirmé |
| Retrait d'une liste autre que « I have » | Le parfum disparaît, sans passer par « I had » | Confirmé (`want`) |
| `del_cust_wr` | Supprime la liste et l'entrée qu'elle contenait | Confirmé |
| CSV du serveur, colonne `Paid` | Montant mis en forme avec la devise : `€ 85,00` | Confirmé |
| Enchaînement `add_list` puis `set_wardrobe_name_notes` | Suffit à créer une liste nommée ; l'icône est facultative | Confirmé |
| Valeur d'une colonne personnalisée dans le JSON (`custom`) | Supposée indexée par l'identifiant de colonne | Non relu |
| Ports | `calendar_month` relu sur trois mois vides. Aucun port créé : écriture publique, laissée en essai à blanc | Lecture confirmée, écriture non rejouée |

Décisions appliquées, d'après les propositions de la section 8 :

- `parfumo_update_collection_item` sans date d'achat ; `copy` sur
  `parfumo_move_collection_item` ; `also_from_had` sur `parfumo_remove_from_collection`.
- `parfumo_log_wear` classé P et gardé en essai à blanc tant que
  `PARFUMO_MCP_PUBLIC_WRITES` n'est pas activé.
- Export : `json` et `csv` depuis ShelfControl, `csv_site` pour le fichier de Parfumo.
- La règle « 200 avec corps vide = erreur » ne s'applique plus aux neuf écritures de la
  section 3.9 : leur succès est prouvé par la relecture.
- Lecture de la collection d'un autre membre : reportée au jalon 4, avec `parfumo_get_user`
  (faite : section 12).

## 11. Mesures du jalon 3 (12 novembre 2026)

Relevées en développant les outils de notes et d'avis. Une connexion au compte de test, une
quarantaine de requêtes, une par seconde au plus, **lectures seules** : aucune note, aucun
vote, aucun texte envoyé. Aucun captcha, aucun blocage. La session a été fermée par
`GET /api/account/logout` en fin d'essai.

| Sujet | Constat | Statut |
| --- | --- | --- |
| `POST /api/review/get_user_review` (`p`, `lang`) | `{"review_title":"","review_text":""}` quand le membre n'a rien écrit | Confirmé (état vide) |
| `POST /api/statement/get_user_statement` (`p`, `lang`) | `{"statement_text":""}` | Confirmé (état vide) |
| Publier un avis | Script du site (`main_login633.js`, `postReview`) : `POST /api/review/create_review` avec `id` (parfum), `title`, `text`, `yt`, `paste_html`. Réponse JSON : `status` 0 et `msg` en cas de refus, sinon `url` | Lu, **jamais envoyé** |
| Publier un statement | `postStatement` : `POST /api/statement/create_statement` avec `id`, `text`. Réponse : `status`, `msg`, `h`, `url` | Lu, **jamais envoyé** |
| Modifier un avis ou un statement | Le formulaire envoie les mêmes champs, sans identifiant de texte : le site est supposé remplacer le texte existant du membre sur ce parfum | **Supposé** |
| Bouton de suppression | Le script du formulaire lit `data-id`, `data-p`, `data-h` sur `.action_delete_review` / `.action_delete_statement`. Le bouton n'est pas servi quand le membre n'a rien écrit | Forme **déduite**, bouton jamais vu |
| Avertissement du formulaire d'avis | « Reviews and submissions must be first-hand and original. Text from AI sources are NOT permitted. Your account will be restricted or deleted if you submit AI generated text. » (affiché au collage d'un texte) | Lu dans le formulaire |
| Panneau de notation | Une échelle non notée a son lien de retrait masqué (`hide`) : c'est ce qui distingue « non noté » d'une note de 0 | Confirmé (fixture du jalon 0 bis) |
| `/Users/{pseudo}/Ratings/{échelle}` | Tris : `d_desc`, `d_asc`, `v_desc`, `v_asc`, `n_*`, `br_*`. Page vide : « No ratings yet » | Confirmé (état vide) |
| `/Users/{pseudo}/Ratings` d'un **autre** membre | **404** une fois connecté : la page est réservée à son propriétaire | Confirmé |
| `/Users/{pseudo}/reviews` | 5 avis par page, texte complet, `?current_page=N`, compteur « 1 - 5 by 20 » | Confirmé (profil public d'un autre membre) |
| `/Users/{pseudo}/statements` | Un bloc par statement, sans date ni lien direct ; l'URL se compose de celle du parfum et de `/statements/{id}` | Confirmé (profil public) |
| Pseudo du membre connecté | Dans l'en-tête de toute page servie à une session (`.nick_name`). Le serveur le lit une fois par session sur `/account/notifications` : en mode cookie il ne connaît pas le pseudo autrement | Confirmé |
| Liaison en mode cookie à travers le serveur | `pf_auth` seul collé sur `/link` : session reconnue, outils du compte utilisables | **Confirmé** (premier essai réel de ce mode) |
| `perfume_classify_panel.php` | Répond en une trentaine de millisecondes sur une connexion déjà ouverte | Mesuré deux fois |

Décisions appliquées, d'après les propositions de la section 8 :

- Sept outils sur les huit du catalogue ; `parfumo_upload_perfume_photo` n'est pas développé.
- Notes, classification, avis et statements sont de type P : ils passent par le même verrou
  que `parfumo_log_wear` (`PARFUMO_MCP_PUBLIC_WRITES`, fermé par défaut).
- La suppression d'un avis ou d'un statement (type D) n'est pas retenue par le verrou.
- Les outils d'écriture d'avis disent à l'assistant que Parfumo interdit les textes produits
  par une IA : il transmet le texte du membre, il ne le rédige pas.
- Les plafonds quotidiens d'écriture publique (section 9 du cahier des charges) restent au
  jalon 4, comme prévu en section 11 (faits : section 12).

Nouvelles fixtures : `auth_user_reviews_empty.html`, `auth_user_statements_empty.html`,
`auth_ratings_empty.html`, `auth_api_get_user_review.json`, `auth_api_get_user_statement.json`
(expurgées), `user_reviews.html`, `user_statements.html` (pages publiques).

## 12. Mesures du jalon 4 (12 novembre 2026)

Relevées en développant les outils sociaux, de profil et de compte. Deux séries de requêtes.

- **Lectures publiques, anonymes** : environ 55 GET (forum et communautés : 16 ; blogs, Souk
  et mur : 29 ; profil et liste d'un membre, scripts publics du site : une dizaine), une par
  seconde au plus, User-Agent du projet. Aucun 403, aucun 429, aucun captcha.
- **Essai connecté, à travers le serveur** : une connexion au compte de test, une cinquantaine
  d'appels d'outils. Écritures réelles : un réglage privé, le parfum signature, le fuseau, un
  abonnement à l'autre compte du commanditaire. Tout a été rétabli, la session fermée par
  `GET /api/account/logout`. **Aucun texte n'a été publié.**

### 12.1 Corrections de ce document

| Sujet | Ce qui était écrit | Ce qui est mesuré |
| --- | --- | --- |
| Recherche du forum (2.1) | `/s_forum_x.php` avec `q` | Le paramètre de texte est **`filter`**. `q` est ignoré et renvoie tous les messages. Page suivante : `current_page=N` ; 20 résultats par page, 1000 au plus ; chaque résultat est un message |
| Recherche du Souk (2.1) | sens des champs non vérifié | `s_type` = `offer`, `need`, `sharing` ; `filter` = texte (vide : aucun résultat) ; `img=0`, valeur par défaut du formulaire, ne garde que les entrées avec photo (le client envoie `img=1`) ; `as` et `cs` valent toujours 1, rôle non établi |
| Communautés (2.1) | `/communities` | `/communities` est le fil de tous les billets ; l'annuaire est **`/communities/discover`**, 12 par page, `?page=N` |
| Mur (3.6) | écriture par `_post_comment.php` | Confirmé dans le script ; les **entrées du mur sont dans la page** `/Users/{pseudo}/Wall` (20 par page, `current_page`), pas chargées à part |
| Liste des abonnements (3.6) | `/account/follow/list` | Page relevée : lignes `.user-row`, onglets All / Active / Inactive / Follow you ; sans abonnement, la page n'a pas d'onglets (« No Users found ») |
| Réglages (3.7) | « chaque champ porte `data-field` et son état » | Une case porte `checked` ; un choix porte `selected` sur son option courante (la fixture du jalon 0 bis avait été expurgée de cette marque) |

### 12.2 Constats nouveaux

| Sujet | Constat | Statut |
| --- | --- | --- |
| Identifiant numérique d'un membre | Dans la page du profil : `#comments-id_cards-{id}`, le bouton « Post to Wall » (`writeComment('#comment-f-id_cards-{id}-0')`) et, connecté, `data-item-id` du bouton « Subscribe & Follow ». **La page anonyme d'un membre au mur vide ne porte pas le premier** (constaté sur un profil) ; l'outil essaie les deux autres, puis la page servie à la session | Constaté une fois |
| Liste d'un autre membre | `/Users/{pseudo}/Collection/{liste}` appelle `getWardrobe(id, pseudo, type, type_url, …, total)` ; `get_wardrobe.php` renvoie toute la liste d'un coup, un `div.war_it` par parfum (`data-id`, `data-wr-id`, `alt` = fin de l'URL du parfum, sans la marque). Nom, marque et lien : `wardrobe_sneak_peek.php`, une requête par entrée | Confirmé (anonyme) |
| État d'un abonnement | `POST /api/subscription_sub_types` (`type=users`, `item_id`) → tableau JSON des abonnements actifs (`["follow"]`). Sert à relire l'effet de `/api/subscription`, qui répond par un corps vide | Confirmé connecté |
| Formulaire du mur | La page d'un profil servie à une session porte `.data("conf", {id, comment_id:'0', type:'id_cards', redirect, p1, p2, np, l, h})` : ce que `sendComment` envoie à `_post_comment.php` (`a`, `id`, `comment_id`, `text`, `p1`, `p2`, `redirect`, `l`, `h`). Réponse : JSON `{type, val, redirect}`, `val` vide en cas de succès | Formulaire confirmé connecté ; envoi **jamais fait** |
| Forum : identifiants | `forum_id` et `topic_id` sont dans `<script type="application/json" id="forum_context">` des pages de catégorie et de fil. Catégorie : `/forums/{slug}/{page}`, 50 fils par page ; fil : `/forums/topic/{slug}/{page}`, 25 messages ; `/forums/post/{id}` sert la page du fil qui contient le message | Confirmé (anonyme) |
| Forum : nouveau fil | `compose_panel.php` (`forum_id`) sert le formulaire au compte de test ; `forum_id` relu : 18 pour « Perfume Discussions » | Confirmé connecté ; envoi **jamais fait** |
| Blog | Fils `/blogs`, `/blogs/news` (8 par page), `/blogs/top`, `/blogs/hot` (semaine, sans pagination) ; blog d'un membre : 6 par page ; commentaires dans la page de l'article | Confirmé (anonyme) |
| Souk : entrée | Listes et résultats ne portent ni état, ni prix, ni envoi, ni note : seulement la page `/Users/{pseudo}/Souk/Item/{id}` (ou `/Souk/Sharing/{id}`) | Confirmé (anonyme) |
| Souk : écriture | Script `soukPanel` : `POST /api/souk/souk_adder` ou `souk_update` avec les champs du formulaire ; réponse JSON `{result:"success", id, url}` ou `{errors:{champ:message}, error}`. Types : `normal_need`, `sample_need` (formulaire), `normal_offer` (script) ; `sample_offer` est **supposé** | Lu, **jamais envoyé** |
| Souk : retrait d'une entrée | Aucune route dans les scripts ni les formulaires servis à un compte sans entrée | **Inconnu** |
| Communautés | Billet : identifiant dans `var postId` ; un billet peut n'avoir aucun bloc de texte (titre et parfum seulement) ; commentaires absents de la page sans session | Confirmé (anonyme) |
| Notification | Script `notifications633.js` : éléments `.notification-item` avec `data-id` et `data-link` ; marquer comme lue : `notification/delete` | Forme **lue, jamais vue** |
| Messagerie : pagination | `_get_postbox_conv_more.php?n=&f=25` par pas de 25 ; `_get_postbox.php?r=&before=&before_date=&l=` | Lu dans `postbox633.js` ; états vides seulement à ce jalon |
| Parfum signature | `account/signature_scent` avec `p` = `p_id` puis `clear` : relu sur le profil à chaque fois | Confirmé connecté |
| Réglage | `_aa_account.php` avec `a=time_zone_id` et `a=hide_note_pics` : relus sur leur page | Confirmé connecté |

### 12.3 Décisions appliquées

- 19 outils : les 13 du catalogue 7.5, `parfumo_communities` (lecture seule) et les 5 du
  catalogue 7.6. Total : 48.
- `parfumo_update_profile` au périmètre réduit proposé en section 8 : genre, pays, fuseau,
  préférences olfactives, parfum signature.
- `parfumo_souk_manage_offer` crée et modifie ; le retrait n'est pas développé.
- `parfumo_update_settings` refuse la langue du compte (`reg_lang`) : les analyseurs lisent
  le site anglais.
- Message privé, abonnement, mur, forum, blog, Souk et profil sont de type P : même verrou
  que les jalons 2 et 3, plus un plafond quotidien par outil et par utilisateur, étendu aux
  outils P de ces deux jalons. Valeurs : README.
- Liste facultative de destinataires (`PARFUMO_MCP_WRITE_RECIPIENTS`) pour les écritures
  vers un autre membre.
- Les textes ne sont jamais rédigés par le serveur ni par l'assistant : les outils le disent.

### 12.4 Ce qui reste incertain après le jalon 4

1. Réponse réelle des envois de mur, de forum, de blog et du Souk : connue par les scripts
   seulement. Format attendu du texte du forum et du blog (texte enrichi ou brut).
2. Étape de publication d'un article après « Save & Preview ».
3. Forme d'une notification, d'une conversation à plusieurs pages, d'une liste
   d'abonnements à plusieurs pages.
4. Route de retrait d'une entrée du Souk ; type `sample_offer`.
5. Modération, quotas et refus du site sur les écritures publiques.

### 12.5 Fixtures

34 nouvelles : `forum_*.html` (6), `communities.html`, `community*.html` (3 en tout),
`blog_*.html` (5), `souk_*.html` (13), `wall_*.html` (2), `user_profile.html`,
`user_collection_list.html`, `frag_user_wardrobe.html` (pages publiques, `csrf_key`
anonyme remplacé), `auth_follow_list.html` et `auth_follow_list_empty.html` (expurgées
comme les autres `auth_*`).

## 13. Mesures du jalon 5 (12 novembre 2026)

Relevées en essayant le test canari (`parfumo-mcp-server canary`, détail dans
[`MAINTENANCE.md`](MAINTENANCE.md)). Trois passages, **lectures seules** : un sur les pages
publiques (27 contrôles), deux sur les pages publiques et le compte de test (48 contrôles),
soit environ 160 requêtes, une par seconde au plus, User-Agent du projet (plus trois lectures
publiques pour l'essai du journal d'audit). Une connexion au
compte de test, liée au serveur en mode cookie (`pf_auth` seul), fermée par
`GET /api/account/logout`. Aucun 403, aucun 429, aucun captcha.

| Sujet | Constat | Statut |
| --- | --- | --- |
| Structure du site | Les 48 lectures dont dépendent les outils répondent comme aux jalons 0 à 4 : aucun analyseur n'a signalé de changement | Confirmé |
| Durée d'un passage complet | 68 secondes pour 48 contrôles | Mesuré une fois |
| Profil d'un compte aux listes vides | La page `/Users/{pseudo}` ne montre aucune liste de collection | Confirmé (compte de test) |
| Export CSV d'une liste vide | Une seule ligne, l'en-tête | Confirmé |
| Billet de communauté | Peut n'avoir aucun texte (déjà noté en 12.2) : 0 caractère sur le billet lu | Confirmé |
| Entrée du Souk | Les champs d'une offre (état, contenance, prix, envoi) sont bien dans la page de l'entrée : 4 champs lus sur l'offre du jour | Confirmé |
| Mode cookie | Deuxième liaison réelle par `pf_auth` seul, réussie ; le cookie est révoqué par la déconnexion finale | Confirmé |

Aucune écriture n'a été envoyée à Parfumo pendant ce jalon.
