# Journal des jalons — prouvé, non prouvé

Relevés écrits à la fin de chaque jalon, du plus récent au plus ancien, conservés tels quels.
Synthèse à jour, en anglais : [`STATUS.md`](STATUS.md).

> **Noms.** Les crates ont été renommés depuis : le binaire local `parfumo-mcp-stdio` de ces
> relevés s'appelle aujourd'hui `parfumo-mcp`, le serveur HTTP `parfumo-mcp` s'appelle
> `parfumo-mcp-server`, et les outils sont dans la bibliothèque `parfumo-mcp-core`. Les tests
> nommés `hypothetical_*` ont été renommés : la connexion qu'ils supposaient a été mesurée
> depuis. La fonction `auth` de `rmcp`, activée mais jamais utilisée, a été retirée.
>
> **Corrections depuis ces relevés.** Deux points notés « non prouvé » plus bas sont levés :
> l'intégration continue tourne sur GitHub à chaque push, et elle construit l'image Docker du
> serveur HTTP puis vérifie sa taille (moins de 50 Mo), son démarrage et sa mémoire au repos
> (`scripts/check-image.sh`). Reste non prouvé : `docker compose up` sur un VPS, derrière Caddy.

## État du binaire stdio : prouvé, non prouvé

Relevé du 12 novembre 2026, même conteneur de développement. `parfumo-mcp-stdio` n'ajoute
aucun outil : pour le rendre possible, `parfumo-mcp` est devenu une bibliothèque en plus d'un
binaire, et les outils reconnaissent leur appelant sans passer par une requête HTTP.

### Prouvé

| Point | Preuve |
| --- | --- |
| Tests | `cargo test --workspace` : 195 tests, 0 échec, sans réseau (7 nouveaux ; les 188 du serveur HTTP passent inchangés). `cargo clippy --workspace --all-targets -- -D warnings` : propre |
| Mêmes outils | Test : `tools/list` du binaire stdio rend les 48 noms du serveur HTTP, avec schémas et annotations. MCP Inspector 2.14.0 en ligne de commande, sur le binaire `release` : 48 outils |
| Compte lu dans l'environnement | Tests sur le vrai binaire, face à un faux site : aucune requête au démarrage ; mode identifiants, une seule connexion, session reprise au lancement suivant sans nouvelle connexion ; reconnexion seule à l'expiration ; mot de passe refusé jamais renvoyé, même après redémarrage ; variable corrigée, compte remplacé ; variables retirées, compte effacé ; mode cookie sans aucune connexion |
| Démarrage refusé | Test : identifiants à moitié fournis, identifiants et cookie ensemble, cookie illisible. L'erreur ne répète aucun secret |
| Secrets | Test : ni mot de passe ni cookie de session sur la sortie standard, sur l'erreur standard ou en clair dans les fichiers locaux ; fichiers en mode 600 |
| Protocole | Test : la sortie standard ne porte que du JSON-RPC ; fermer l'entrée standard arrête le binaire proprement (code 0) |
| **En direct**, mode cookie | Binaire `release` sur le vrai site, compte de test, cookie `pf_auth` seul : `parfumo_session_status` (session valide), `parfumo_list_collections` (7 listes), `parfumo_get_perfume` (sans compte), `parfumo_log_wear` verrou fermé (essai à blanc, rien envoyé). Lectures seules ; session fermée ensuite |
| Journal d'audit | `parfumo-mcp audit`, pointé sur la base locale, lit les appels du binaire stdio |
| Taille | Binaire `release` de 10,0 Mo (serveur HTTP : 11,0 Mo) |

### Non prouvé

- **Mode identifiants en direct** : la connexion par `PARFUMO_USER` et `PARFUMO_PASSWORD` n'a
  tourné que face au faux site. Le code de connexion est celui du serveur HTTP, prouvé en
  direct au jalon 2 ; l'essai réel du binaire stdio s'est fait en mode cookie pour ne pas
  ajouter une connexion au compte de test.
- **Aucun vrai client MCP** (Claude Code, Claude Desktop) n'a lancé le binaire : seulement
  MCP Inspector en ligne de commande et les tests. Inspector négocie la version `2025-11-25`.
- **Windows et macOS** : jamais compilé ni lancé ailleurs que sous Linux. Le chemin par défaut
  de la base sous Windows (`%LOCALAPPDATA%`) et l'absence de mode 600 hors d'Unix sont écrits,
  pas essayés.
- **L'intégration continue n'a jamais tourné** (pas de dépôt distant), et l'image Docker n'a
  pas été reconstruite depuis que `crates/` contient un troisième crate : le `Dockerfile` est
  inchangé et ne compile que `parfumo-mcp`.
- Les écritures : aucune n'a été envoyée par le binaire stdio. Elles passent par le même code
  que celles du serveur HTTP ; leurs points non prouvés restent ceux des jalons 2 à 4.

## État au jalon 5 : prouvé, non prouvé

Relevé du 12 novembre 2026, même conteneur de développement (sans Docker). Le jalon 5
n'ajoute aucun outil : il durcit le serveur. Constats et corrections de l'audit :
[`docs/AUDIT.md`](AUDIT.md) ; exploitation : [`docs/MAINTENANCE.md`](MAINTENANCE.md).

### Prouvé au jalon 5

| Point | Preuve |
| --- | --- |
| Tests | `cargo test --workspace` : 188 tests, 0 échec, sans réseau (15 nouveaux). `cargo clippy --workspace --all-targets -- -D warnings` : propre |
| Canari, **en direct** | `parfumo-mcp canary` sur le vrai site, compte de test relié en mode cookie : 48 contrôles `ok` (27 publics, 21 du compte), 68 secondes, code de sortie 0, aucune écriture. Rapport enregistré, relu par `canary --last`, et visible dans `/healthz` |
| Canari, par test | Site factice dont toutes les pages ont changé : statut `changed`, contrôles dépendants `skipped`, alerte reçue par un webhook local. Captcha ou limitation : arrêt immédiat. Compte factice : aucune écriture envoyée, aucun secret dans le rapport |
| Journal d'audit | Test : la somme des écritures du journal égale le nombre de requêtes d'écriture reçues par le site factice ; un essai à blanc compte zéro. **En direct** : `audit --stats`, `--errors`, `--writes`, `--json` sur un serveur réel après quatre appels |
| Corrections de l'audit | Neuf constats, tous corrigés, sept avec un test dédié : identifiants refusés jamais rejoués, verrou par adresse, rejeu d'un jeton de rafraîchissement, redirections de `parfumo_fetch_page`, page de consentement, enregistrement de clients borné |
| Mémoire | 20 Mo de mémoire résidente au repos, binaire `release` de 10,8 Mo |
| Surcoût par appel | 2,7 ms en médiane (11,9 ms au pire) sur 30 appels d'un outil sans requête vers Parfumo |
| Migration | Test : une base d'avant le jalon 5 reçoit la colonne `writes` à l'ouverture, ses lignes sont conservées |
| Compte de test | Deux passages du canari, lectures seules ; session fermée par `GET /api/account/logout` |

### Non prouvé au jalon 5

- **L'image Docker n'a jamais été construite et `docker compose up` jamais lancé** : il n'y a
  pas de Docker dans le conteneur de développement. Le `Dockerfile`, `docker-compose.yml` et
  le `Caddyfile` sont relus et durcis, pas exécutés. Le critère « `docker compose up` sur un
  VPS vierge » de la section 10 reste à prouver ; `scripts/check-image.sh`, à lancer sur le
  VPS, mesure la taille (moins de 50 Mo attendus, environ 20 Mo estimés), le démarrage et
  la mémoire.
- **L'intégration continue n'a jamais tourné** : le dépôt n'a pas de dépôt distant.
- **Le déclenchement quotidien du canari** n'a pas été observé (il faudrait attendre l'heure) :
  le calcul de l'heure est testé, le passage a été lancé à la main. L'alerte par webhook n'a
  été reçue que par un serveur local de test.
- **Le verrou par adresse** dépend de l'en-tête `X-Forwarded-For` posé par Caddy : non
  essayé derrière un vrai Caddy.
- Cloudflare depuis l'adresse du VPS, client OAuth réel, second client MCP : inchangé.
- Les points non prouvés des jalons 2 à 4 (écritures à texte, Souk, états non vides) restent
  tels quels : ce jalon n'a rien écrit sur Parfumo.

## État au jalon 4 : prouvé, non prouvé

Relevé du 12 novembre 2026, même conteneur de développement. Le jalon 4 ajoute 19 outils
(sections 7.5 et 7.6 du cahier des charges, plus `parfumo_communities`) et les plafonds
d'écriture publique, appliqués aussi aux outils des jalons 2 et 3. Détail et sorties réelles :
[`docs/TOOLS.md`](TOOLS.md#prouvé-et-non-prouvé-jalon-4) ; mesures du site :
[`docs/DISCOVERY.md`](DISCOVERY.md), section 12.

### Prouvé au jalon 4

| Point | Preuve |
| --- | --- |
| Tests | `cargo test` : 173 tests, aucun accès réseau. 54 nouveaux : analyse de 34 nouvelles fixtures (profil, liste d'un autre membre, messagerie, abonnements, réglages, forum, communautés, blogs, Souk, mur) et outils de bout en bout contre un faux site |
| Verrou | Test : sans `PARFUMO_MCP_PUBLIC_WRITES`, message, abonnement, mur, forum, blog, Souk et profil n'envoient aucune requête d'écriture et ne consomment aucun quota |
| Plafonds | Tests : plafond par outil et par utilisateur, erreur `rate_limited` avec délai, essai à blanc toujours possible, plafond 0, compte conservé après redémarrage sur la même base, outils des jalons 2 et 3 plafonnés aussi. **En direct** : plafond de 2 atteint sur `parfumo_manage_follow`, troisième appel refusé sans rien envoyer, compte retrouvé après redémarrage |
| Destinataires | Test et **en direct** : un membre hors liste est refusé avant toute requête |
| Lectures, en direct | Les 13 outils de lecture, sur le vrai site : profil et listes d'un membre, boîte de messages, notifications, abonnements, mur, forum (catégories, fils, recherche, lecture), blogs, Souk (listes, recherche, entrée, Souk d'un membre), communautés, profil, réglages, état de session |
| Écritures réelles, en direct | `parfumo_update_settings` (réglage activé puis rétabli), `parfumo_update_profile` (parfum signature posé puis retiré, fuseau changé puis rétabli), `parfumo_manage_follow` (abonnement à l'autre compte du commanditaire, puis désabonnement). Chaque état a été relu sur le site |
| Écritures à texte, à blanc en direct | Message, mur, forum (réponse et nouveau fil), blog, demande au Souk : destinataire ou cible résolus, formulaire et jeton lus sur le vrai site, charge utile construite, rien envoyé |
| Contenu tiers | Tests : un message reçu qui tente de fermer son délimiteur reste enfermé ; les textes de membres sont signalés comme non fiables |
| Compte de test | Rendu à son état initial : aucun abonnement, aucun parfum signature, réglages rétablis, session fermée |

### Non prouvé au jalon 4

- **Aucun texte n'a été publié.** L'envoi réel d'un message privé, d'un message de mur, d'un
  message de forum, d'un article de blog et des lignes de préférences olfactives exigerait un
  texte. Parfumo interdit les textes produits par une IA, et aucun texte du titulaire du
  compte n'a été fourni : rien n'a été rédigé pour l'occasion. Ces routes sont donc prouvées
  jusqu'à la charge utile, pas au-delà. Pour lever ce point : fournir un texte écrit par le
  titulaire pour chaque outil.
- **Souk** : une demande fictive serait un faux contenu montré aux autres membres, et les
  offres sont refusées au compte de test (compte récent). Le retrait d'une entrée n'est pas
  développé : sa route n'a jamais été vue.
- **États non vides jamais vus en direct** : une conversation, un message, une notification.
  La messagerie repose sur les fixtures du jalon 0 bis ; la forme d'une notification est lue
  dans le script du site.
- **Forum et blog** : le texte part en texte brut, alors que les éditeurs du site produisent
  du texte enrichi ; le rendu n'est pas vérifié. Le formulaire de blog s'intitule « Save &
  Preview » : l'étape de publication n'est pas connue.
- **Genre et pays** : même route que le fuseau, prouvé ; non rejoués.
- **Non développés** : écritures des communautés (rejoindre, commenter), fil d'activité des
  abonnements, modification d'un article de blog, avatar. Les commentaires d'un billet de
  communauté ne sont pas servis sans session.
- Modération, quotas et messages de refus de Parfumo : jamais déclenchés.
- MCP Inspector n'a pas été relancé ; `tools/list` (48 outils) est couvert par les tests.
- Image Docker, Cloudflare depuis le VPS, vrai client OAuth : inchangé depuis le jalon 1.

## État au jalon 3 : prouvé, non prouvé

Relevé du 12 novembre 2026, même conteneur de développement. Le jalon 3 ajoute sept outils :
`parfumo_rate_perfume`, `parfumo_classify_perfume`, `parfumo_list_my_ratings`,
`parfumo_write_review`, `parfumo_write_statement`, `parfumo_delete_review_or_statement`,
`parfumo_list_my_reviews`. `parfumo_upload_perfume_photo` n'est pas développé (retrait
proposé au jalon 0 bis).

### Prouvé au jalon 3

| Point | Preuve |
| --- | --- |
| `cargo build --release` | Passe. Binaire de 9,4 Mo |
| `cargo test --workspace` | 119 tests, 0 échec, sans réseau : 38 d'analyse sur fixtures, 26 de transport contre un faux serveur, 55 côté serveur |
| `cargo clippy --workspace --all-targets -- -D warnings` | Propre |
| Verrou des écritures publiques | Test : sans `PARFUMO_MCP_PUBLIC_WRITES`, noter, classer, publier un avis ou un statement n'envoie aucune requête d'écriture ; l'outil montre la charge utile |
| Lectures, en direct | Sur le compte de test, à travers `/mcp` : mes notes, mes avis, mes statements (listes vides), libellés et votes de classification d'un parfum, notes actuelles d'un parfum, avis et statement actuels d'un parfum |
| Écritures, en direct, verrou fermé | Les quatre outils de type P ont répondu `dry_run`. Suppression : `absent`, rien envoyé |
| `parfumo_rate_perfume`, **prouvé en direct** | Verrou ouvert, compte de test : deux notes envoyées, relues sur « mes notes », retirées, disparition vérifiée. Relevé dans `docs/TOOLS.md` |
| `parfumo_classify_perfume`, **prouvé en direct** | Verrou ouvert : deux votes envoyés, relus, retirés, disparition vérifiée. Relevé dans `docs/TOOLS.md` |
| Mode cookie | Compte de test relié sur `/link` avec `pf_auth` seul : premier essai réel de ce mode, réussi |
| Isolation | Test à deux comptes : notes, statements et écritures cloisonnés |
| Audit | Test : l'entrée d'un avis porte le parfum et la longueur du texte, pas le texte |
| Jeton de suppression | Test : le jeton `h` du bouton est envoyé à Parfumo et n'apparaît dans aucune réponse d'outil |

### Non prouvé au jalon 3

- **`parfumo_write_review`, `parfumo_write_statement` et
  `parfumo_delete_review_or_statement` : non prouvés en direct.** Publier, modifier et
  supprimer un avis ou un statement n'ont jamais été envoyés par personne : la charge utile
  vient du script du site, le reste est supposé (republier remplace le texte, forme du
  succès, motifs de refus, forme du bouton de suppression). Détail dans `docs/TOOLS.md` et
  `docs/DISCOVERY.md` §11.
- **Pourquoi : Parfumo interdit les textes produits par une IA** dans les avis et statements
  et restreint ou supprime les comptes concernés. Un texte de preuve rédigé par l'assistant
  en est un : il n'a pas été publié. La preuve demande deux textes écrits par le titulaire
  du compte. Le serveur ne peut pas vérifier qui a écrit un texte.
- Notes et classification : prouvées sur un parfum et deux échelles (`scent`,
  `longevity`) ; les trois autres échelles n'ont pas été envoyées.
- Les plafonds quotidiens d'écriture publique (10 avis ou statements) sont prévus au jalon 4
  (faits depuis : voir plus haut).
- MCP Inspector n'a pas été relancé à ce jalon ; `tools/list` (29 outils) est couvert par
  les tests.

## État au jalon 2 : prouvé, non prouvé

Relevé du 12 novembre 2026, même conteneur de développement.

### Prouvé au jalon 2

| Point | Preuve |
| --- | --- |
| `cargo build --release` | Passe. Binaire de 9,7 Mo |
| `cargo test --workspace` | 104 tests, 0 échec, sans réseau : 31 d'analyse sur fixtures (dont ShelfControl et le suivi des ports, capturés connecté), 26 de transport contre un faux serveur, 47 côté serveur |
| `cargo clippy --workspace --all-targets -- -D warnings` | Propre |
| Connexion réelle | Compte de test relié par la page `/link` en mode identifiants : une seule connexion, réussie |
| Collection, en direct | Sur le compte de test, à travers `/mcp` : lecture des listes, ajout (essai à blanc, réel, répété), mise à jour (contenance, prix, devise, contenance restante, note privée), lecture, export `json`, `csv` et `csv_site`, statistiques, liste personnalisée (création, copie dedans, renommage, masquage, affichage, suppression), déplacement, retrait (« I have » vers « I had », puis retrait complet). Chaque écriture a été relue sur le site |
| État final du compte de test | Sept listes à zéro, aucune liste personnalisée : tout a été annulé |
| Ports, en direct | `parfumo_list_wears` (période vide) ; `parfumo_log_wear` et `parfumo_delete_wear` en essai à blanc seulement |
| MCP Inspector 2.14.0 | `tools/list` : 22 outils ; `tools/call` sur un outil du compte |
| Isolation | Test à deux comptes sur les outils du compte : listes, écritures, cookies et journal d'audit cloisonnés ; un utilisateur ne peut ni modifier ni retirer le flacon d'un autre |
| Reconnexion | Test : session expirée côté Parfumo, l'appel suivant (lecture ou écriture) se reconnecte seul et réussit ; mot de passe changé : `reauth_required` |
| Secrets | Journal du serveur réel après l'essai : ni identifiants Parfumo, ni clé maîtresse, ni jeton porteur, ni jeton de 32 caractères hexadécimaux |
| Mémoire | 20 Mo de mémoire résidente après l'essai |

### Non prouvé au jalon 2

- **Enregistrer et supprimer un port pour de vrai.** Un port est visible sur le profil
  public : `parfumo_log_wear` reste en essai à blanc jusqu'à la décision du commanditaire.
  Le parcours réel (enregistrement, date, commentaire, relecture, suppression) est testé
  contre un faux serveur qui suit les réponses relevées au jalon 0 bis. Trois points restent
  des hypothèses : le suivi `/assistant/tracker` liste bien le port qui vient d'être créé,
  la page n'est pas paginée, et `delete_iwear` ignore un identifiant inconnu.
- **Valeurs des colonnes personnalisées.** L'écriture (`columns.php`, `op=value`) a été
  confirmée au jalon 0 bis, mais aucune ligne avec une valeur n'a été relue : la forme du
  champ `custom` (clé = identifiant de colonne) est une déduction.
- **Reconnexion réelle.** Aucune session n'a expiré pendant l'essai ; la reconnexion n'a
  tourné que contre le faux serveur.
- **Collection d'un autre membre.** `parfumo_get_collection` ne lit que le compte relié. La
  lecture par `get_wardrobe.php` est reportée au jalon 4, avec `parfumo_get_user`.
- **Listes longues.** Le compte de test n'a jamais eu plus d'un flacon : pagination,
  découpage de l'export et plafond de taille ne sont prouvés que par les tests.
- **Image Docker, Cloudflare depuis le VPS, vrai client OAuth** : inchangé depuis le jalon 1.

## État au jalon 1 : prouvé, non prouvé

Relevé du 11 novembre 2026, dans le conteneur de développement (Rust 1.99.1, sans Docker).

### Prouvé

| Point | Preuve |
| --- | --- |
| `cargo build --release` | Passe sans avertissement. Binaire de 9,2 Mo (cible `x86_64-unknown-linux-gnu`) |
| `cargo test` | 80 tests, 0 échec, sans réseau : 25 d'analyse sur 29 fixtures HTML capturées sur le site, 21 de transport contre un faux serveur, 34 côté serveur |
| `cargo clippy --workspace --all-targets -- -D warnings` | Propre |
| `/healthz` | Répond `{"status":"ok","version":"0.1.0"}` sur le binaire `release` lancé à la main |
| Les 10 outils | Chacun appelé pour de vrai sur www.parfumo.com à travers le serveur ; sorties dans [`docs/TOOLS.md`](TOOLS.md) |
| Pagination et plafond | En direct : curseur de recherche, 67 parfums renvoyés pour `limit=100`, 14 avis pour `limit=50`, chaque fois avec `next_cursor` |
| Erreurs normalisées | En direct : `not_found` et `invalid_input`. Les autres codes : tests seulement |
| MCP Inspector | Version 2.14.0, mode ligne de commande : `tools/list` (10 outils) et `tools/call` sur les 10 outils. Cet Inspector négocie la version **`2025-11-25`** |
| Spécification `2026-07-28` | Un `tools/call` sans état, avec les en-têtes `MCP-Protocol-Version`, `Mcp-Method` et `Mcp-Name`, envoyé par `curl` au binaire `release`, plus les tests d'intégration. Pas de client tiers `2026-07-28` essayé |
| OAuth 2.1 PKCE | Flux complet joué avec `curl` sur le binaire `release` : métadonnées, 401 avec `WWW-Authenticate`, enregistrement dynamique, page de connexion, code, échange PKCE, jeton accepté sur `/mcp`, code rejoué refusé. Même flux en test d'intégration, avec rotation du jeton de rafraîchissement |
| Jeton porteur statique | Utilisé pour tous les appels réels |
| Chiffrement AES-256-GCM | Tests : aller-retour, mauvaise clé, blob d'un autre utilisateur, altération |
| Isolation | Test à deux utilisateurs : cookies, secrets, journal d'audit et liaison cloisonnés ; les outils publics n'envoient aucun cookie de compte |
| Secrets | Test : aucun mot de passe, jeton ni cookie dans les journaux, le journal d'audit ou les réponses d'outil. Vérifié aussi sur le journal du serveur réel |
| Mémoire | 39 Mo de mémoire résidente après les appels réels |

### Implémenté mais non prouvé

Les deux points « Connexion à Parfumo » et « Mode cookie » ci-dessous datent du jalon 1.
Ils sont levés depuis : la connexion a été mesurée au jalon 0 bis (cookie `pf_auth`) et
utilisée pour de vrai au jalon 2. Le mode cookie a été essayé à travers le serveur au jalon 3 (liaison par `pf_auth` seul, réussie).

- **Image Docker et `docker compose up`.** Docker n'est pas installé dans le conteneur de
  développement : ni `docker build`, ni la taille de l'image, ni `docker compose up` n'ont
  été exécutés. Le `Dockerfile` (Alpine, binaire musl, utilisateur non root 10001) et les
  fichiers Compose sont écrits mais jamais lancés. La taille attendue, environ 20 Mo, est
  une estimation : image Alpine plus un binaire de 9 Mo.
- **Connexion à Parfumo.** Aucun compte de test n'existe. Le mode identifiants, le
  renouvellement de session et la détection de l'état connecté reposent sur une hypothèse :
  une connexion réussie redirige ailleurs que vers la page d'erreur et pose les cookies
  `user_id` et `user_id_key`. La logique (chiffrement, trois tentatives, `reauth_required`,
  arrêt sur mot de passe refusé ou captcha) est testée contre un faux serveur, dans des tests
  nommés `hypothetical_*`. Aucune connexion réelle n'a été tentée.
- **Mode cookie.** La liste exacte des cookies à importer n'est pas connue. La page accepte
  l'en-tête `Cookie` entier.
- **Client ID Metadata Documents.** Les règles de validation sont testées ; la récupération
  d'un document chez un vrai client ne l'a pas été.
- **OAuth avec un vrai client MCP** (Claude, Inspector en mode graphique) : non essayé. Le
  critère « deux clients MCP différents » de la section 10 reste à faire.
- **Comportement de Cloudflare depuis l'adresse du VPS** : inconnu. Aucun défi reçu depuis
  le conteneur de développement.
- **Tris de recherche** `durability_desc`, `sillage_desc`, `bottle_desc`, `pricing_desc`,
  `popularity_asc` : non appelés.

### Écarts constatés avec `docs/DISCOVERY.md`

`docs/DISCOVERY.md` n'a pas été modifié (sa mise à jour relève du jalon 0 bis). Ce que le
jalon 1 a mesuré de différent :

- Pas de classement « top » général : `/Perfumes/Tops` répond 404, il n'existe que par
  genre. `/Perfumes/Trends` contient trois classements. D'où les valeurs de `ranking`.
- Pas de filtre de recherche par concentration : le formulaire n'a pas ce champ.
- `get_reviews.php` avec `full=Ja` renvoie les textes complets ; `get_reviews_text.php` est
  inutile.
- `get_similar.php` ne donne aucun lien vers les parfums : l'URL s'obtient par
  `similar_sneak_peek.php`, une requête par parfum.
- Un corps vide en réponse 200 est ambigu : c'est aussi la réponse normale pour un parfum
  sans avis ou sans similaires. Il est traité en erreur seulement quand le total connu est
  supérieur à zéro.
- Une note simple est sous `/Fragrance_Note/{nom}`, une catégorie sous
  `/Fragrance_Notes/{nom}`.
- Les `label_filters_*` exigent un paramètre `input` précis, sinon corps vide.
- Les identifiants d'URL sont irréguliers (`Franois_Demachy`, `sauvage-elixir`) : le client
  essaie l'identifiant déduit puis l'autocomplétion du site.

### Limites connues

- Le premier appel de `parfumo_get_similar_perfumes` pour un parfum prend une dizaine de
  secondes (une dizaine de requêtes à une par seconde), puis le cache sert 24 h.
- Depuis le jalon 2, la limite d'une requête par seconde est commune aux outils publics et à
  la session du compte d'un même utilisateur.
- La fonction `auth` de `rmcp` est activée comme le demande le cahier des charges, mais elle
  ne sert qu'aux clients : le serveur d'autorisation OAuth est écrit dans `parfumo-mcp`.
