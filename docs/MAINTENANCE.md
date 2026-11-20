# Maintenance — exploiter le serveur et le réparer quand Parfumo change

Ce document dit quoi faire dans quatre situations : le canari signale un changement du
site, un outil renvoie `parser_outdated`, Parfumo bloque le serveur, ou il faut exploiter
le serveur au quotidien (journal d'audit, sauvegarde, mise à jour).

Parfumo ne publie aucune API. Le serveur lit les pages et les appels internes du site : ils
changent sans préavis. Tout ce qui dépend de leur forme est dans un seul fichier,
`crates/parfumo-client/src/site.rs`.

Depuis la version 0.3.0, quand un compte est relié en mode identifiants (identifiant et mot
de passe), une partie des lectures de la base de données passe par l'interface de
l'application de Parfumo (`/app/v1`), avec le jeton de session de ce membre. Ce qui dépend
de la forme de ses réponses est dans les fichiers `crates/parfumo-client/src/app*.rs`. Elle
est décrite dans [APP_API.md](APP_API.md) (en anglais), avec les règles que le code suit.
Les pages web répondent à tout le reste, et à tout pour un compte relié par cookie ou sans
compte.

## 1. Le test canari

Une fois par jour, le serveur lit sur le vrai site chaque type de page et de route dont les
outils dépendent, et vérifie que les analyseurs y trouvent ce qu'ils attendent.

| Sujet | Valeur |
| --- | --- |
| Heure | `PARFUMO_MCP_CANARY_HOUR`, heure UTC de 0 à 23. Défaut : 4. `off` coupe le canari |
| Zones | Trois, dans cet ordre : `public` (les pages web, sans jeton), `interface` (les lectures faites par l'interface de l'application, avec le jeton de session du compte de test), `account` (les pages du compte sur le site web, avec ses cookies et sans jeton) |
| Pages publiques (`public`) | 27 contrôles : recherche, fiche, avis, statements, similaires, marque, parfumeur, note, classements, page libre, forum, communautés, blogs, Souk, profil, mur, avis et liste d'un membre |
| Interface de l'application (`interface`) | 9 contrôles, seulement si le compte de test est relié en mode identifiants : il détient alors un jeton de session. Pseudo du membre, recherche, fiche, marque, parfumeur, année de sortie, parfums similaires, notes du membre sur un parfum, classification du membre. Les contrôles de la fiche, de la marque et du parfumeur comprennent la lecture de la page web qui complète la réponse (libellé de rang, pays) : une page introuvable ou qui ne se lit plus donne `CHANGED` ; un captcha ou une limitation de débit sur cette page donne `FAILED` et arrête le passage ; une page injoignable (5xx, délai dépassé, réseau) donne `FAILED` et le passage continue. Rien n'est demandé à `/app/v1` sans jeton |
| Compte de test (`account`) | 21 contrôles de plus si `PARFUMO_MCP_CANARY_USER` nomme un utilisateur du serveur dont le compte Parfumo est relié : listes, export CSV, ports, notes, avis, classification, messagerie, notifications, abonnements, réglages, profil. Tous sont lus sur le site web, sans le jeton : le pseudo, le panneau de notes et le panneau de classification restent ainsi contrôlés par les analyseurs HTML, qui servent les comptes reliés par cookie |
| Écritures | Aucune. Le canari ne fait que lire |
| Débit | Celui des outils : une requête par seconde. Une soixantaine de requêtes pour les zones `public` et `account` ; 68 secondes mesurées le 12 novembre 2026. La zone `interface` en ajoute une quinzaine ; sa durée n'a pas été mesurée sur le vrai site |
| Arrêt | Immédiat sur un captcha ou une limitation de débit, du site web comme de l'interface de l'application (page de blocage, réponse 429, à un appel ou à la connexion, blocage déjà connu du processus) : le contrôle en cours est `FAILED`, le reste est noté `skipped`, zone `account` comprise |
| Alerte | `PARFUMO_MCP_CANARY_WEBHOOK` : une requête POST en JSON quand le résultat n'est pas `ok`, et une autre quand il redevient `ok`. Le corps porte la même phrase sous `text` et `content` (formats de Slack et de Discord) et la liste des contrôles en cause |
| Trace | Table `canary_runs` (90 jours), journal du serveur (niveau `ERROR` par contrôle en cause), et `"canary"` dans `/healthz` |

Utilisez un compte Parfumo de test pour `PARFUMO_MCP_CANARY_USER`, pas un compte personnel :
créez un utilisateur du serveur (`user add canari`), reliez-le au compte de test sur `/link`,
puis mettez `PARFUMO_MCP_CANARY_USER=canari` dans `.env`.

**Reliez ce compte en mode identifiants** (identifiant et mot de passe, le formulaire
principal de `/link`). C'est la seule façon de contrôler `/app/v1` : un compte relié par
cookie n'a pas de jeton de session, et le canari ne demande alors rien à l'interface.
Gardez ce compte **sans note et sans vote de classification sur Dior Sauvage Eau de
Toilette**, le parfum que le canari lit : avec une note de zéro ou un vote de classification
sur ce parfum, l'interface laisse la lecture au panneau du site web, et les contrôles
`own rating` et `own classification` sont notés `n/a` au lieu d'être faits.

| Compte du canari | Zone `interface` | Résultat quand rien n'a changé |
| --- | --- | --- |
| Relié en mode identifiants, jeton de session détenu, sans note ni vote sur le parfum lu | Les 9 contrôles | 57 contrôles `ok` |
| Relié par cookie | Une seule ligne, `n/a` (`application session`). Rien n'est envoyé à `/app/v1` | 48 `ok` et 1 `not applicable` ; statut `ok`, pas d'alerte |
| Relié en mode identifiants, sans jeton | Une seule ligne, `FAILED` (`application session`) : la connexion à l'interface n'a pas donné de jeton (le journal du serveur dit pourquoi), ou n'est pas encore retentée après un échec. Si le processus sait l'adresse bloquée, ou si cette connexion reçoit la page de blocage ou un 429, la ligne dit `the application interface asks to wait (…)` et le passage s'arrête : la zone `account` est `skipped` | Statut `inconclusive`, alerte envoyée |
| Utilisateur du canari sans compte relié | Une seule ligne, `skipped`, en plus de la ligne `FAILED` `account session` de la zone `account` | Statut `inconclusive`, alerte envoyée |
| Aucun (`PARFUMO_MCP_CANARY_USER` vide, ou `--public-only`) | Pas de zone | 27 contrôles `ok` |

**Jeton refusé pendant un passage.** Si l'interface refuse le jeton (réponse 401), le
contrôle en cours est `FAILED`. Le jeton est oublié aussitôt, pour le canari comme pour les
outils du même utilisateur, et n'est plus jamais envoyé. Le serveur se reconnecte une fois à
l'interface, et les contrôles suivants se font avec le nouveau jeton. Un second refus arrête
la zone : le reste est `skipped`. Sans nouveau jeton, le reste de la zone est `skipped`
aussi. Le statut est `inconclusive` dans tous ces cas. Le refus est reconnu même si un appel
d'outil du même utilisateur a renouvelé la session avant la fin de la lecture. Si le compte
est relié de nouveau, ou délié, pendant une lecture, ce contrôle est `skipped` (`the account
was linked again or unlinked while this read ran …`) : la lecture n'est pas jugée. Une
session que le site ferme pendant un passage est oubliée dans la base à la fin du passage.
Lancée à la main à côté du serveur, la commande est un autre processus : le refus, puis le
nouveau jeton, sont enregistrés dans la base, et le serveur les reprend à son appel suivant
pour cet utilisateur (colonne `app_rev`), sans se reconnecter. Seul un appel d'outil déjà
commencé dans le serveur au moment du refus peut encore envoyer une fois l'ancien jeton.

**L'interface demande d'attendre pendant un passage.** Si l'interface répond par sa page de
blocage ou par une limitation de débit (429), ou si le processus sait déjà l'adresse
bloquée, le contrôle en cours est `FAILED` (`the application interface asks to wait (…)`)
et le passage s'arrête : plus rien n'est envoyé, ni à l'interface ni au site web, et la zone
`account` est `skipped`. La réponse que le site web a donnée à la place de l'interface n'est
pas jugée. Le statut est `inconclusive`. Il en va de même quand le compte n'a pas de jeton
au début de la zone, ou quand la connexion qui renouvelle la session après un refus reçoit
un 429 : la ligne `application session`, ou le contrôle en cours, est `FAILED` avec la même
phrase.

```sh
docker compose exec server parfumo-mcp-server canary --last     # dernier rapport enregistré
docker compose exec server parfumo-mcp-server canary            # un passage tout de suite
docker compose exec server parfumo-mcp-server canary --public-only --json
```

La commande sort avec le code 0 si le statut du passage est `ok`, 1 sinon. Lancée à la main
pendant que le serveur tourne, elle a son propre limiteur : elle ajoute au plus une requête
par seconde à celles du serveur. Elle partage avec lui la session de l'interface
enregistrée dans la base : le jeton renouvelé par l'un est repris par l'autre. Ne la lancez
pas en boucle.

### Lire un rapport

Extrait du rapport d'un compte relié par cookie : la ligne de résumé, puis les lignes qui ne
sont pas `ok` (la ligne de date, les lignes `ok` et une partie du dernier détail sont
omises ici).

```
canary changed: 46 ok, 1 changed, 0 failed, 1 skipped, 1 not applicable (public pages and the Parfumo account of `canari`)
CHANGED  public     perfume                          290 ms  parser outdated: rating block  [get_perfume]
skipped  public     perfumer                           0 ms  needs the perfumer of the `perfume` check  [get_perfumer]
n/a      interface  application session                0 ms  the Parfumo account is linked by the cookie of a browser, […]: nothing is asked of /app/v1  [open_app_session]
```

La ligne de résumé ne porte « , N not applicable » que si N est supérieur à zéro : sans
contrôle `n/a`, elle est celle des versions précédentes.

| Résultat d'un contrôle | Dans le rapport JSON | Sens | Que faire |
| --- | --- | --- | --- |
| `ok` | `"ok"` | La route répond et l'analyseur trouve ce qu'il attend | Rien |
| `CHANGED` | `"changed"` | La structure a changé : `parser outdated`, page introuvable, ou page lue mais vide de ce qu'elle doit contenir (`parsed, but …`). Dans la zone `interface` : aussi une lecture que l'interface n'a pas servie elle-même, ou une page web qui complète sa réponse devenue introuvable ou illisible (avertissement `ranking unavailable…`, `country unavailable…`) | Section 2 |
| `FAILED` | `"failed"` | Pas de verdict : Parfumo injoignable, captcha, limitation (du site web, d'une page web qui complète une réponse de l'interface, ou de l'interface elle-même : page de blocage, 429), session du compte refusée, jeton de l'interface refusé, ou compte relié en mode identifiants sans jeton | Section 3 |
| `skipped` | `"skipped"` | Non lancé : il dépend d'un contrôle qui n'a pas passé, ou le passage (ou la zone) a été arrêté. Ou lancé mais non jugé : le compte a été relié de nouveau pendant la lecture | Réparer d'abord le contrôle dont il dépend |
| `n/a` | `"not_applicable"` | Ne s'applique pas au compte tel qu'il est : relié par cookie, il n'a pas de jeton de session ; avec une note de zéro ou un vote de classification sur le parfum lu, l'interface laisse cette lecture au panneau du site web, lu dans la zone `account`. C'est l'état durable d'une installation saine | Rien. Pour que l'interface soit contrôlée : relier le compte en mode identifiants, sans note ni vote sur ce parfum |

Statut du passage : `changed` dès qu'un contrôle est `CHANGED` ; sinon `inconclusive` s'il
reste des `FAILED` ou des `skipped` ; sinon `ok`. Un contrôle `n/a` ne compte pas dans le
statut et ne déclenche aucune alerte. Le nom entre crochets est la fonction de
`parfumo-client` derrière le contrôle : c'est le point d'entrée de la réparation.

Un canari `ok` ne prouve pas tout. Il ne voit ni les écritures, ni les états que le compte
de test n'a pas (une notification, une conversation, une liste remplie). Avec un compte
relié par cookie, il ne voit pas non plus l'interface de l'application.

## 2. Quand le site change

Symptômes : le canari dit `CHANGED`, ou un outil renvoie `parser_outdated`, ou une liste
revient vide alors qu'elle ne devrait pas.

1. **Trouver la fonction.** Le rapport du canari la nomme. Pour un outil, `parfumo-mcp-server audit
   --errors --since 24h` donne l'outil et la cible ; le message `parser_outdated` nomme le
   bloc manquant.
2. **Regarder la page réelle.** Une requête à la fois, par le transport de production :

   ```sh
   cargo run -q -p parfumo-client --example live -- perfume Dior/Sauvage_Eau_de_Toilette
   # page ou route connectée, sur le compte de test (identifiants dans .env) :
   cargo run -q -p parfumo-client --features explore --example explore -- login
   cargo run -q -p parfumo-client --features explore --example explore -- req GET /account/notifications -o notifications
   ```

   Les corps sont écrits dans `/tmp/parfumo-explore/out/`, hors du dépôt. L'outil `explore`
   refuse les routes de suppression de compte, de mot de passe et d'e-mail.
3. **Corriger `crates/parfumo-client/src/site.rs`**, et lui seul si possible : `routes`
   (URL), `sel` (sélecteurs CSS), `re` (expressions régulières), `markers` (textes repères).
   Si la forme des données a changé, `parse.rs` ou `parse/social.rs` suit. Pour un contrôle
   de la zone `interface`, le code est ailleurs : voir plus bas.
4. **Remplacer la fixture** dans `crates/parfumo-client/tests/fixtures/` par la nouvelle
   page (section suivante), puis adapter le test qui la lit (`tests/parse_fixtures.rs`,
   `forum_fixtures.rs`, `market_fixtures.rs`, `social_fixtures.rs`).
5. **Vérifier** : `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D
   warnings`, `cargo fmt --all --check`, puis `parfumo-mcp-server canary` contre le vrai site.
   Les tests à plusieurs threads de `app_session_tests.rs` passent par `on_four_threads` :
   leur limite de temps est comptée sur le thread du test, parce qu'un
   `tokio::time::timeout` ne se déclenche pas quand tous les threads du runtime attendent un
   verrou `std`. En CI, chaque job de test est arrêté au bout de 30 minutes
   (`timeout-minutes`).
6. **Noter le changement** dans `docs/DISCOVERY.md` (une ligne : ce qui était écrit, ce qui
   est mesuré) et déployer (section 5).

Si une fonction du site a disparu, l'outil doit le dire : retirez-le de la liste ou faites-lui
renvoyer une erreur claire, plutôt que de le laisser renvoyer du vide.

### Un contrôle de la zone `interface`

Le rapport nomme entre crochets la même fonction que dans la zone `public` (`get_perfume`,
`get_brand`…) : c'est celle que les outils appellent. Avec un jeton de session, cette
fonction passe par une fonction des fichiers `crates/parfumo-client/src/app_*.rs`, et c'est
là que se corrige un changement de l'interface :

| Contrôle (fonction du rapport) | Appel de l'interface | Fonction à corriger | Fixture rejouée par les tests |
| --- | --- | --- | --- |
| `own nickname` (`own_nick`) | `GET /auth/me` | `app_nick` (`app.rs`), `app_own_nick` (`app_own.rs`) | `app_own_me.json` |
| `search` (`search_perfumes`) | `GET /perfumes` | `app_search_perfumes` (`app_db.rs`) | `app_perfumes_text.json`, `app_perfumes_brand.json`, `app_search_creed.json` |
| `perfume` (`get_perfume`) | `GET /perfume_details/{numéro}` | `app_perfume_record`, `app_perfume_by_number` (`app_db.rs`) | `app_perfume_details.json` |
| `brand` (`get_brand`) | `GET /brand_details` | `app_get_brand` (`app_brand.rs`) | `app_brand_details.json` |
| `perfumer` (`get_perfumer`) | `GET /perfumer_details` | `app_get_perfumer` (`app_perfumer.rs`) | `app_perfumer_details.json` |
| `release year` (`get_rankings`) | `GET /perfumes` avec `y_0` | `app_rankings` (`app_rankings.rs`) | `app_rankings_year.json` |
| `similar perfumes` (`get_similar_perfumes_with`) | `GET /similar` | `app_similar_perfumes` (`app_similar.rs`) | `app_similar_list.json` |
| `own rating` (`my_ratings_of`) | `GET /rating` | `app_my_ratings_of` (`app_own.rs`) | `app_own_rating.json`, `app_own_rating_none.json` |
| `own classification` (`my_classification_of`) | `GET /classification` | `app_my_classification_of` (`app_own.rs`) | `app_own_classification.json` |

Trois points propres à cette zone :

- **Le détail dit de quel côté est le changement.** `parser outdated: /app/v1/…` ou
  `parsed, but …` sur une partie de la réponse : la forme de la réponse de l'interface a
  changé. `the reader warns: ranking unavailable…`, `country unavailable…` ou `country and
  founding year unavailable…` : la page web qui complète la réponse (fiche, marque,
  parfumeur) est introuvable ou ne se lit plus ; la correction est alors celle d'une page
  web, dans `site.rs`. Un captcha, une limitation ou une panne sur cette page ne donne pas
  cet avertissement dans le canari : le contrôle est `FAILED`, avec l'erreur de la page. `the application interface did
  not answer this read (the web site did), or its answer changed` : la réponse est venue du
  site web à la place de l'interface, ou la réponse de l'interface n'a plus ce qui la
  distingue (numéro du parfum dans chaque ligne, numéro de la marque dans la fiche, votes de
  chaque parfum similaire). L'interface laisse une lecture au site web quand sa recherche
  ne trouve pas la référence : vérifiez cela avant de toucher au code. Quand elle demande
  d'attendre (adresse bloquée, 429), le canari ne juge pas la lecture : le contrôle est
  `FAILED` et le passage s'arrête (section 3).
- **Les fixtures** sont les fichiers `crates/parfumo-client/tests/fixtures/app_*.json`,
  lus par `tests/app_db.rs`, `app_brand.rs`, `app_perfumer.rs`, `app_rankings.rs`,
  `app_similar.rs` et `app_own.rs`. Le changement se note dans `docs/APP_API.md`, pas dans
  `docs/DISCOVERY.md`.
- **Rien n'est demandé à `/app/v1` sans jeton de session**, pas même pour regarder une
  réponse : quelques requêtes sans jeton valide font bloquer l'adresse cinq minutes. L'outil
  `explore` refuse toute requête vers `/app/v1` qui ne porte pas d'en-tête `Authorization`,
  sauf la connexion. Le jeton est celui du compte de test, jamais une autre clé.

Aucun réglage ne coupe l'interface. En attendant une correction, un compte relié par cookie
lit tout sur les pages web.

### Recapturer une fixture

Une fixture est la copie d'une réponse réelle. Les tests n'ont jamais accès au réseau.

| Type | Capture | Nettoyage obligatoire avant de l'ajouter au dépôt |
| --- | --- | --- |
| Page publique | `explore req GET <chemin> -o <nom>` sans s'être connecté (ou `curl` avec le User-Agent du projet) | Remplacer le `csrf_key` anonyme (16 caractères) par une valeur factice ; pseudos des membres → `member001`, `member002`… (partout : liens, textes, attributs) ; photos de profil (`user_photo/…`) → `00/00_0123456789abcdef…` ; identifiants `u_id` → `2000001`, `2000002`… |
| Page ou fragment connecté (`auth_*`) | `explore login` puis `explore req …` | Pseudo du compte → `testuser` ; pseudo d'un autre membre → `otheruser` ; identifiants numériques de membres → `1000001`, `1000002` ; tout jeton de 32 caractères hexadécimaux ou plus → `0123456789abcdef…` ; adresse e-mail et pays retirés |
| Réponse d'API (`auth_api_*.json`) | `explore req POST /api/… k=v` | Même règle |
| Réponse de l'interface de l'application (`app_*.json`) | Une requête `GET` vers `/app/v1/…`, toujours avec le jeton de session du compte de test, jamais sans jeton | Pseudo du compte → `testuser` ; pseudos d'autres membres → `member1`, `member2`… ; adresse e-mail → adresse factice ; identifiant numérique du compte → `1` ; aucun jeton de session dans le fichier |

Une page servie à une session porte toujours le pseudo du compte et le `csrf_key` de la
session, même si elle paraît publique : ne l'enregistrez jamais telle quelle. Le test
`auth_fixtures_are_scrubbed` échoue si une fixture `auth_*` contient un jeton réel ; il ne
détecte pas un pseudo oublié, et il ne lit pas les fixtures `app_*` : relisez-les à la main.
Les réponses de l'interface au sujet d'un parfum mêlent des données publiques et celles du
compte qui demande (ses listes, ses votes).

Règles de l'exploration, inchangées : une requête par seconde au plus, trois tentatives de
connexion au plus, arrêt au premier captcha, jamais `/account/delete` ni les routes de mot
de passe ou d'e-mail. Les écritures visibles par d'autres membres ne sont pas rejouées pour
une fixture : leur charge utile se lit dans le formulaire et le script du site. Aucun texte
n'est rédigé pour l'occasion : Parfumo interdit les textes produits par une IA.

Terminez par `explore req GET /api/account/logout` et supprimez `/tmp/parfumo-explore`.

## 3. Quand Parfumo refuse

| Symptôme | Cause probable | Que faire |
| --- | --- | --- |
| `captcha_required` | Défi Cloudflare ou Turnstile. Le serveur ne le résout jamais et attend 15 minutes avant de réessayer pour cet utilisateur | Attendre. Si cela dure, l'utilisateur se connecte dans son navigateur et relie son compte en mode cookie sur `/link`. Depuis une adresse de centre de données, le défi peut être permanent : c'est le risque n° 3 du cahier des charges |
| `reauth_required`, statut `reauth_required` dans `user list` | Parfumo a refusé le mot de passe enregistré (changé, ou compte restreint). Le serveur n'envoie plus rien pour ce compte | L'utilisateur relie son compte à nouveau sur `/link` |
| `reauth_required`, statut `expired` | Mode cookie : le cookie `pf_auth` n'est plus accepté (déconnexion dans le navigateur d'origine) | Recoller un cookie sur `/link` |
| `reauth_required`, statut `login_failed` | Trois connexions de suite ont échoué sans refus du mot de passe (Parfumo indisponible). Nouvel essai automatique après 15 minutes | Attendre ; vérifier que parfumo.com répond |
| `rate_limited` avec un délai | Plafond quotidien d'écriture publique du serveur, ou 429 de Parfumo | Respecter le délai. Ne pas relever le débit : une requête par seconde par utilisateur, deux au total |
| `rate_limited` avec un délai en secondes, 300 au plus, sur une lecture | Parfumo bloque l'adresse du serveur : sa page de blocage (« Access Denied … try again in … », statut 404) a été rencontrée. Pendant cinq minutes à partir de la première rencontre, et pour tous les utilisateurs du processus, plus rien n'est envoyé à `/app/v1`, connexion comprise : les lectures passent par les pages web, sans erreur pour l'utilisateur. L'erreur `rate_limited` n'apparaît que si une page web montre elle aussi la page de blocage ; elle donne alors les secondes restantes | Attendre la fin du délai, sans rien relancer. Quand ce blocage a été observé, il suivait des requêtes vers `/app/v1` sans jeton valide |
| Canari, zone `interface` : `CHANGED` avec `the application interface did not answer this read (the web site did), or its answer changed` | L'interface a laissé la lecture au site web (recherche sans résultat), ou sa réponse a changé | Relancer un passage plus tard, une fois. Si le détail revient : section 2 |
| Canari, zone `interface` : `FAILED` avec `the application interface asks to wait (the site blocks the address of this server)` ou `(it rate limited a call)`, sur un contrôle ou sur la ligne `application session`, le reste `skipped` avec `run stopped after …: the application interface asks to wait` | L'interface a répondu par sa page de blocage ou par une limitation de débit (429), à un appel ou à la connexion, ou le processus savait déjà l'adresse bloquée. Le canari n'a plus rien envoyé | Attendre la fin du blocage (cinq minutes), sans rien relancer, puis relancer un passage, une fois |
| Journal du serveur, niveau `WARN` : `the application interface refused the credentials: the web pages are used until the account is linked again` | L'interface de l'application a refusé l'identifiant et le mot de passe à sa connexion. Ils ne lui sont plus envoyés, redémarrage compris (colonne `app_refused` de `parfumo_links`). Le compte reste relié et les outils lisent les pages web. `parfumo_session_status` ne le montre pas : seul ce journal le dit. Si c'est le compte du canari : ligne `FAILED` `application session` (avec `the application interface asks to wait…` tant qu'un blocage de l'adresse est connu du processus) | L'utilisateur relie son compte à nouveau sur `/link` |
| Journal du serveur, niveau `WARN` : `no session of the application interface: the web pages are used instead` | La connexion à l'interface a échoué sans refus des identifiants (panne, réponse 400, 403, 404 ou 422, réponse sans jeton). Nouvel essai au premier appel d'outil après 15 minutes | Attendre. Les outils lisent les pages web entre-temps |
| Journal du serveur, niveau `WARN` : `the renewed session of the application interface was refused: the web pages are used for a while` | Une session renouvelée après un refus a été refusée de nouveau dans les 15 minutes. Aucune nouvelle connexion à l'interface avant la fin de ce délai, redémarrage compris (colonnes `app_ended` et `app_renewed_at`), et quel que soit le processus qui a renouvelé la session | Attendre : le serveur réessaie seul après ce délai. Les outils lisent les pages web entre-temps |
| `upstream_error` répété | Parfumo en panne, ou 403 | `canary --public-only` dit si le site répond encore au serveur |

Le serveur ne contourne rien : pas de rotation d'adresse, pas de résolution de captcha, pas
d'imitation de navigateur. Son `User-Agent` est fixe et dit ce qu'il est. Vers l'interface
de l'application, il n'envoie que le jeton de session du membre, obtenu avec ses propres
identifiants : la clé propre à l'application n'est jamais utilisée.

## 4. Journal d'audit et compteurs

Chaque appel d'outil laisse une ligne : date, utilisateur, outil, cible, résultat, durée, et
`writes`, le nombre de requêtes d'écriture réellement envoyées à Parfumo. Un essai à blanc,
une lecture et un refus valent `writes=0`. Le journal ne contient ni texte d'avis ou de
message (seulement sa longueur), ni mot de passe, ni cookie, ni jeton. Conservation : 90 jours.

```sh
parfumo-mcp-server audit                                  # 50 derniers appels
parfumo-mcp-server audit --writes --since 7d              # ce qui a vraiment été écrit sur Parfumo
parfumo-mcp-server audit --user alice --tool parfumo_send_message
parfumo-mcp-server audit --errors --since 24h             # appels en erreur
parfumo-mcp-server audit --stats --since 24h              # par outil : appels, erreurs, écritures, durée
parfumo-mcp-server audit --json --limit 1000              # une ligne JSON par appel
```

Dans Docker, préfixez par `docker compose exec server`. Le journal du serveur
(`docker compose logs server`) est en JSON, une ligne par appel d'outil, sans secret ;
Docker le garde par tranches de 10 Mo, cinq au plus.

## 5. Exploitation courante

**Sauvegarde.** Deux choses, à garder séparément : la clé maîtresse (`PARFUMO_MCP_MASTER_KEY`
dans `.env`) et la base SQLite du volume `parfumo-data`. Sans la clé, les identifiants et les
cookies enregistrés (et, rangé avec eux, le jeton de session de l'interface de
l'application) sont illisibles et chaque utilisateur doit relier son compte à nouveau.

```sh
docker compose stop server
docker run --rm -v parfumo-mcp_parfumo-data:/data -v "$PWD":/backup alpine \
  tar czf /backup/parfumo-data-$(date +%F).tar.gz -C /data .
docker compose start server
```

Le nom du volume est préfixé par celui du dossier du projet (`docker volume ls`). Pour
restaurer : arrêter, extraire l'archive dans le volume, démarrer.

**Mise à jour.**

```sh
git pull                      # nouveautés des fichiers Compose et du Caddyfile
docker compose pull           # télécharge l'image publiée
docker compose up -d
curl https://<votre-domaine>/healthz
```

Le serveur utilise l'image publiée `ghcr.io/hessbey/parfumo-mcp-server`. Sans réglage, c'est
la dernière version ; `PARFUMO_MCP_VERSION=0.3.0` dans `.env` en fige une, et la mise à jour
consiste alors à changer ce numéro avant les commandes ci-dessus. Pour une image construite
soi-même, voir [HOSTING.md](HOSTING.md#install).

Le schéma de la base se met à jour seul au démarrage. Les sessions Parfumo et les jetons
OAuth survivent au redémarrage. De la 0.2.x à la 0.3.0, rien d'autre n'est à faire : la
base est mise à niveau sur place (quatre colonnes ajoutées à la table `parfumo_links` :
`app_refused`, `app_ended`, `app_renewed_at`, `app_rev`), il n'y a ni nouveau réglage ni nouvelle
variable d'environnement, et les comptes restent reliés. Un compte relié en mode
identifiants reçoit sa session de l'interface de l'application au premier appel d'outil.

**Secrets.**

| Geste | Commande | Effet |
| --- | --- | --- |
| Clé d'accès perdue ou divulguée | `user rotate-key <nom>` | L'ancienne clé est refusée tout de suite et toutes les sessions OAuth de l'utilisateur sont révoquées. La clé n'expire jamais d'elle-même : elle sert à la fois de jeton porteur, sur la page `/link` et sur la page de connexion OAuth |
| Départ d'un utilisateur | `user remove <nom>` | Secrets, sessions, jetons et compteurs effacés |
| Clé maîtresse dans un secret Docker | `PARFUMO_MCP_MASTER_KEY_FILE=/run/secrets/…` | La clé n'est plus dans l'environnement du conteneur |

Durées des jetons OAuth : accès une heure, rafraîchissement trente jours, à usage unique.
Un jeton de rafraîchissement présenté deux fois révoque tous les jetons issus de la même
autorisation : le client légitime doit alors se reconnecter. C'est voulu.

**Pare-feu.** Seuls les ports 80 et 443 de Caddy sont publiés. Le port 8080 du serveur ne
sort pas du réseau Docker.

**Surveillance.** `GET /healthz` répond 200 tant que la base répond, et donne le dernier
verdict du canari : `{"canary":{"at":…,"status":"ok"},"status":"ok","version":"0.3.0"}`. Le
verdict du canari ne fait jamais échouer le contrôle de santé : un conteneur ne doit pas
redémarrer parce que Parfumo a changé une page.

## 6. Publier une version

Une version se publie en poussant un tag. Le reste est automatique
(`.github/workflows/release.yml`).

1. Mettre le numéro de version à jour dans `Cargo.toml` (`[workspace.package]`, champ
   `version`), puis lancer `cargo check` pour que `Cargo.lock` suive. Valider et pousser sur
   `main`, et attendre que l'intégration continue soit verte.
2. Poser le tag du même numéro, précédé de `v`, et le pousser :

   ```sh
   git tag v0.3.0
   git push origin v0.3.0
   ```

   Le tag doit correspondre exactement à la version de `Cargo.toml`, sinon la publication
   s'arrête avant de rien produire.
3. En une dizaine de minutes, la publication :
   - compile les deux binaires pour Linux (Intel/AMD et ARM), macOS (Apple silicon et Intel)
     et Windows, et vérifie qu'ils démarrent ;
   - crée la page de version sur GitHub, avec les archives et le fichier `SHA256SUMS` ;
   - publie l'image Docker du serveur pour amd64 et arm64 sur
     `ghcr.io/hessbey/parfumo-mcp-server`, sous le numéro de version et sous `latest` ;
   - publie le binaire local sur npm : le paquet `parfumo-mcp`, que lance
     `npx -y parfumo-mcp`, et un paquet par système qui contient le binaire
     (`@hessbey/parfumo-mcp-linux-x64`, `-linux-arm64`, `-darwin-arm64`, `-darwin-x64`,
     `-win32-x64`).

Un tag de pré-version (`v0.3.0-rc.1`) crée une pré-version et ne déplace pas `latest` ; sur
npm, il est publié sous l'étiquette `next`.

**Version 0.3.0.** Le tag `v0.2.3` n'a jamais été posé. Le tag `v0.3.0` est donc la première
version publiée qui porte les deux changements : celui de la 0.2.3 (aucune page n'est
conservée, chaque lecture interroge Parfumo) et l'interface de l'application.

**npm.** La publication sur npm vient en dernier, après la page de version : si elle échoue,
le reste est déjà en ligne. Corriger la cause, puis « Re-run failed jobs » sur l'exécution :
les paquets déjà publiés sont sautés. Elle s'authentifie de deux façons, dans cet ordre :

- **Sans jeton** (publication de confiance) : npm vérifie que la demande vient bien du
  fichier `release.yml` de ce dépôt. Aucun secret, rien à renouveler. Il faut pour cela que
  **chacun des six paquets** désigne ce dépôt comme éditeur de confiance : sur npmjs.com,
  page du paquet → Settings → « Trusted Publisher » → GitHub Actions, avec l'utilisateur
  `hessbey`, le dépôt `parfumo-mcp`, le fichier `release.yml`, aucun environnement, et
  `npm publish` coché dans « Allowed actions » (pas `npm stage publish`, qui attendrait une
  approbation à la main sur npmjs.com pour chaque paquet de chaque version).
- **Avec un jeton**, en secours, si la première façon échoue et que le secret `NPM_TOKEN`
  existe dans le dépôt (Settings → Secrets and variables → Actions). L'exécution affiche
  alors un avertissement. C'est aussi la seule façon de publier un paquet qui n'existe pas
  encore sur npm. Le jeton se crée sur npmjs.com (Access Tokens → Generate New Token) :
  droits « Read and write » sur tous les paquets, option « Bypass two-factor
  authentication » cochée. npm limite sa durée à 90 jours.

Les six paquets : `parfumo-mcp`, et `@hessbey/parfumo-mcp-linux-x64`, `-linux-arm64`,
`-darwin-arm64`, `-darwin-x64`, `-win32-x64`.

**Noms des paquets.** Le paquet principal garde le nom nu `parfumo-mcp`. Les paquets des
systèmes, que personne ne tape, sont publiés sous le compte npm du projet (`@hessbey/…`) : à
la version 0.2.1 ils portaient des noms nus, et npm a accepté les quatre paquets Linux et
macOS puis refusé `parfumo-mcp-win32-x64` (« Package name triggered spam detection »), si
bien que le paquet principal n'est jamais sorti. Ces quatre paquets orphelins de la 0.2.1 ont
été supprimés de npm le jour même.

Jusqu'à la version 0.3.0, une copie du paquet principal était aussi publiée sous le nom
`@hessbey/parfumo-mcp`, au cas où npm refuserait le nom nu. npm l'a accepté à chaque
version : la copie n'est plus publiée depuis. Ses versions déjà publiées restent sur npm.

Les paquets se construisent avec `npm/build.mjs` et se vérifient sans rien publier avec
`node npm/test.mjs target/release/parfumo-mcp`, ce que fait l'intégration continue sur les
trois systèmes.

**La première fois**, l'image publiée est privée : dans GitHub, onglet Packages du compte,
ouvrir `parfumo-mcp-server`, puis Package settings → Change visibility → Public.

Les binaires ne sont pas signés (voir [LOCAL.md](LOCAL.md#install)).

## 7. Avant de livrer une modification

- [ ] `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings` et
      `cargo fmt --all --check`
- [ ] Toute nouvelle route ou sélecteur est dans `site.rs`, avec sa fixture nettoyée
- [ ] Tout nouvel outil d'écriture : `dry_run`, relecture de l'état, envoi par `Clients::send`
      (c'est ce qui le compte dans le journal d'audit), verrou et plafond s'il est public
- [ ] `parfumo-mcp-server canary` : `ok` sur le vrai site
- [ ] `scripts/check-image.sh` sur une machine qui a Docker
