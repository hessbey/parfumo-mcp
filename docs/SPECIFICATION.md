# Cahier des charges — Serveur MCP Parfumo

11 novembre 2026

> **Document d'origine, conservé tel quel.** C'est le cahier des charges rédigé avant le
> développement, pas un mode d'emploi. Plusieurs choses ont changé depuis :
>
> - le serveur HTTP s'appelle `parfumo-mcp-server`, et `parfumo-mcp` désigne aujourd'hui le
>   programme local. Les commandes d'administration sont donc `parfumo-mcp-server user add`,
>   `audit`, `canary`… ;
> - un utilisateur n'a plus de nom ni de mot de passe à saisir, seulement une clé d'accès ;
> - le code est réparti en quatre crates.
>
> Les commandes à utiliser sont dans [HOSTING.md](HOSTING.md) et [LOCAL.md](LOCAL.md) ;
> l'état réel du projet est dans [STATUS.md](STATUS.md).

## 1. Objectif

Construire un serveur MCP distant, écrit en Rust, qui permet à un assistant IA de consulter Parfumo et d'agir sur un compte Parfumo au nom de son propriétaire. Le périmètre visé est l'ensemble des fonctions du site : base de données, collection, notes et avis, social, profil.

Ce document est destiné à un agent de code. Il fixe les décisions, les contraintes, le catalogue d'outils attendu et les critères d'acceptation. Tout ce qui dépend du fonctionnement réel du site est renvoyé à une phase d'exploration obligatoire (section 4), car Parfumo ne documente aucune API publique.

L'agent doit vérifier au démarrage les versions citées ici (spécification MCP, SDK `rmcp`) : elles évoluent vite.

## 2. Décisions arrêtées

| Sujet | Décision |
| --- | --- |
| Périmètre fonctionnel | Tout : base de données, collection, notes et avis, social, profil et compte |
| Méthode d'accès au site | Choisie par l'agent après la phase d'exploration, avec justification écrite |
| Garde-fous à l'écriture | Aucune confirmation : les outils exécutent directement |
| Langage | Rust |
| Déploiement | Serveur distant en HTTP, image Docker sur un VPS, derrière HTTPS |
| Clients visés | Tout client MCP conforme au standard (pas de dépendance à un client particulier) |
| Connexion à Parfumo | Identifiant et mot de passe stockés chiffrés côté serveur, session renouvelée automatiquement. Import d'un cookie de session proposé aux proches qui ne veulent pas confier leur mot de passe |
| Utilisateurs | Le commanditaire et quelques proches : plusieurs comptes Parfumo isolés |

Deux précisions sur ces choix :

- **Rust.** Le temps de réponse sera dominé par le réseau vers Parfumo, pas par le langage. Le gain réel de Rust est ailleurs : un binaire unique, une image Docker de quelques dizaines de Mo et une faible consommation mémoire sur le VPS.
- **Aucune confirmation.** Une erreur de l'assistant peut publier un avis, envoyer un message ou supprimer une entrée sans retour possible. La section 8 prévoit donc des compensations qui ne ralentissent rien : paramètre `dry_run` optionnel, annotations d'outil, journal d'audit.

* **Accès automatisé assumé.** Le commanditaire veut un accès entièrement automatisé et accepte que son compte Parfumo puisse être suspendu. Les limites de débit et l'absence de contournement de captcha restent en vigueur : elles protègent le site et la durée de vie du serveur.

## 3. Contraintes et risques

Parfumo ne publie pas d'API officielle : une recherche n'a trouvé ni documentation développeur ni programme d'accès. Le serveur reposera donc sur les interfaces que le site utilise pour lui-même, ce qui crée trois risques à traiter dès la conception.

| Risque | Conséquence | Réponse exigée |
| --- | --- | --- |
| Interfaces internes instables | Un changement du site casse des outils sans préavis | Sélecteurs et routes isolés dans un seul module, tests sur fixtures, test canari quotidien, erreur `parser_outdated` explicite |
| Protections anti-robot | Blocage d'IP, captcha, surtout depuis une IP de datacenter | Débit faible. Un captcha est relayé à l'utilisateur, qui le résout lui-même (section 6). Pas de résolution automatique |
| Actions publiques automatisées | Avis, messages ou posts générés en masse assimilables à du spam | Plafonds par utilisateur et par jour sur les écritures publiques (section 9) |

Le serveur est réservé à un usage personnel : chaque utilisateur n'agit que sur son propre compte. Aucune collecte en masse de la base Parfumo, aucune redistribution des données.

Chaque proche est prévenu du risque de suspension avant de relier son compte.

## 4. Phase 0 : exploration (bloquante)

Aucun outil n'est développé avant la validation du rapport d'exploration par le commanditaire. L'objectif est de savoir comment le site fonctionne réellement, puis de choisir la méthode d'accès.

Il existe très probablement des interfaces internes : les appels que les pages web font en arrière-plan, et l'interface utilisée par l'application mobile. Elles ne sont ni publiques ni garanties, mais lorsqu'elles renvoient du JSON elles sont plus fiables que l'analyse du HTML.

Travail attendu :

1. Parcourir parfumo.com connecté avec un compte de test et enregistrer le trafic réseau (fichier HAR) pour chaque fonction de la section 7. C'est ainsi que l'on trouve les interfaces internes.
2. Pour chaque fonction, noter : URL, méthode, paramètres, jeton CSRF éventuel, format de réponse (JSON ou HTML), besoin de JavaScript.
3. Documenter le parcours de connexion : formulaire, cookies, durée de session, double authentification éventuelle.
4. Repérer les protections : type de captcha, moment où il apparaît, défi anti-robot à l'entrée du site. En déduire si le relais de captcha de la section 6 est réalisable.
5. Lister les fonctions réellement présentes sur le site. Le catalogue de la section 7 est une hypothèse : retirer ce qui n'existe pas, ajouter ce qui manque.
6. Vérifier l'export CSV de ShelfControl (vue tableur de la collection) : c'est potentiellement la voie la plus simple pour lire toute une collection.
7. Choisir la méthode d'accès selon l'ordre de préférence ci-dessous.

Ordre de préférence pour chaque fonction :

1. Appel interne renvoyant du JSON, rejoué en HTTP.
2. Requête HTTP classique et analyse du HTML.
3. Navigateur sans interface : exclu de la version 1. Tout doit passer par des requêtes. Si une fonction s'avère impossible sans navigateur, l'agent la signale dans le rapport et le commanditaire décide.

Interdits : faire résoudre un captcha par un service ou un modèle, usurper l'application mobile en extrayant ses secrets, masquer l'origine des requêtes par rotation d'IP.

Livrable : `docs/DISCOVERY.md` avec la carte des routes, la méthode retenue par fonction et la liste finale des outils.

## 4 bis. Jalon 0 bis : exploration connectée par l'agent

L'agent explore lui-même la partie connectée du site, par requêtes HTTP, sur un compte de test fourni par le commanditaire. Cette étape remplace la capture HAR manuelle prévue à la fin de `docs/DISCOVERY.md`.

Le jalon 0 a couvert la partie publique : 17 outils sont prêts, 7 partiels, 25 en attente. Les 16 points de la section 9 de `docs/DISCOVERY.md` forment la liste de travail de ce jalon.

**Fourni par le commanditaire**

- Un compte Parfumo de test, créé à la main (l'inscription est protégée par Turnstile).
- Ses identifiants dans un fichier `.env` : `PARFUMO_TEST_USER` et `PARFUMO_TEST_PASSWORD`. Le fichier est ignoré par Git. Les identifiants n'apparaissent ni dans les journaux, ni dans les fixtures, ni dans le rapport.

**Travail attendu**

1. Se connecter par `POST /api/account/login`. Consigner la redirection, les cookies posés, la présence de `<meta name="csrf-token">` et la durée de session observée.
2. Lire les pages et les scripts servis une fois connecté pour trouver les routes de chaque fonction : collection, ShelfControl et son export, ports, notes, classification, avis, statements, photos, notifications, réglages, profil, abonnements, mur, blog, Souk, forum, communautés, déconnexion.
3. Essayer une fois chaque écriture privée sur le compte de test, relever la requête et la réponse, puis annuler l'essai.
4. Relever le format des erreurs une fois connecté : jeton invalide, quota, corps vide.
5. Enregistrer des fixtures expurgées de chaque réponse utile, pour les tests du jalon 1 et des suivants.
6. Mettre à jour `docs/DISCOVERY.md` : sections 3, 4, 5, 7, 8 et 9, avec la liste finale des outils et les retraits.

**Règles**

- Seul le compte de test est utilisé. Même débit qu'au jalon 0 : une requête par seconde au plus.
- Écritures visibles par d'autres membres (avis, statement, post de forum, mur, blog, offre du Souk, commentaire) : déduire la charge utile du formulaire et des scripts. N'envoyer réellement que si le contenu est neutre et supprimable aussitôt, et le supprimer. Sinon, marquer la route « vue, non testée ».
- Messagerie privée : si elle existe, s'arrêter et demander au commanditaire un pseudo destinataire. Aucun message à un inconnu.
- Trois tentatives de connexion au plus. Si un captcha ou un blocage apparaît, s'arrêter et le rapporter.
- Ne jamais appeler `/account/delete`, ni les routes de changement de mot de passe ou d'e-mail.
- Signaler toute restriction propre aux comptes récents (forum, Souk) : elle fausse le résultat.

**Décisions prises après le jalon 0**

- Le relais de captcha de la section 6 est abandonné : le seul captcha du site est lié au navigateur. L'outil renvoie `captcha_required`, et le repli est l'import de cookie.
- La méthode dominante est la requête HTTP avec analyse du HTML. Le JSON ne sert qu'aux écritures sous `/api/*`.
- Un appel interne qui répond 200 avec un corps vide est traité comme une erreur.

Le jalon 1 peut démarrer en parallèle sur le socle et les 10 outils de base de données, déjà prêts. Point d'arrêt : le rapport mis à jour est validé par le commanditaire avant les jalons 2 à 4.

## 5. Architecture technique

Un seul binaire Rust expose le point d'entrée MCP en Streamable HTTP, parle à Parfumo en HTTP et garde son état dans une base SQLite locale.

| Composant | Rôle | Choix proposé |
| --- | --- | --- |
| Protocole MCP | Outils, schémas, transport | `rmcp`, le SDK Rust officiel, avec les fonctions `server`, `macros`, `transport-streamable-http-server` et `auth` |
| Serveur HTTP | Point d'entrée `/mcp`, pages d'enrôlement, `/healthz` | `axum` + `tokio` |
| Client Parfumo | Requêtes, cookies, reconnexion | `reqwest` avec magasin de cookies par utilisateur, TLS `rustls` |
| Analyse HTML | Extraction des données des pages | `scraper`, sélecteurs regroupés dans un module unique |
| Stockage | Utilisateurs, secrets chiffrés, sessions, cache, audit | SQLite (`sqlx` ou `rusqlite`), fichier sur volume Docker |
| Débit | Limitation par utilisateur et globale | `governor` |
| Schémas | Entrées et sorties typées des outils | `serde` + `schemars` |
| Journalisation | Traces structurées sans secret | `tracing` en JSON |
| Navigateur (optionnel) | Secours pour les pages exigeant JavaScript | Hors version 1. Conteneur Chromium séparé seulement si le commanditaire le décide après la phase 0 |

Version du protocole : viser la spécification MCP `2026-07-28`, la version courante, qui rend le protocole sans état et impose les en-têtes `Mcp-Method` et `Mcp-Name` sur les requêtes HTTP. Accepter aussi `2025-11-25` si le SDK le permet, pour les clients pas encore migrés.

Organisation du code attendue :

- `parfumo-client` : bibliothèque indépendante de MCP (connexion, routes, analyse, modèles). Testable seule.
- `parfumo-mcp` : serveur, authentification, définition des outils, qui appelle la bibliothèque.
- `xtask` ou scripts : capture de fixtures, test canari.

Déploiement : image Docker multi-étapes, utilisateur non root, configuration par variables d'environnement, `docker-compose.yml` fourni avec un reverse proxy assurant HTTPS (Caddy ou Traefik). Le domaine Parfumo est configurable (`PARFUMO_BASE_URL`), avec parfumo.com par défaut.

## 6. Authentification et multi-utilisateurs

Deux authentifications distinctes coexistent : celle du client MCP auprès du serveur, et celle du serveur auprès de Parfumo. Elles ne doivent jamais être confondues.

**Client vers serveur.** Le serveur est exposé sur Internet et donne accès à des comptes : aucune requête anonyme n'est acceptée.

- OAuth 2.1 avec PKCE, conforme au volet autorisation de la spécification MCP, métadonnées de ressource protégée publiées.
- Enregistrement des clients : Client ID Metadata Documents en priorité, enregistrement dynamique conservé pour compatibilité (la spécification `2026-07-28` le déclasse au profit des premiers).
- Repli : jeton porteur statique par utilisateur, pour les clients qui ne gèrent qu'un en-tête `Authorization`.
- Pas d'inscription libre. Les utilisateurs sont créés par l'administrateur en ligne de commande (`parfumo-mcp user add`).

**Serveur vers Parfumo.** Chaque utilisateur relie son propre compte Parfumo.

- Deux modes de liaison, au choix de chaque utilisateur sur une page d'enrôlement HTTPS du serveur. Rien n'est jamais saisi dans la conversation avec l'assistant ni passé en paramètre d'outil.
- Mode identifiants (par défaut) : identifiant et mot de passe Parfumo conservés chiffrés. Le serveur se reconnecte seul à l'expiration de la session, avec trois tentatives au plus, puis erreur `reauth_required`.
- Mode cookie : l'utilisateur colle le cookie de session copié depuis son navigateur. Aucun mot de passe sur le serveur ; à l'expiration, l'outil renvoie `reauth_required` et l'utilisateur recolle un cookie.
- Chiffrement au repos (AES-256-GCM ou équivalent), clé maîtresse fournie par variable d'environnement ou secret Docker, absente de la base.
- Un utilisateur peut délier son compte : identifiants et cookies sont alors effacés.

**Captcha.** Le serveur ne résout jamais un captcha lui-même. Quand Parfumo en présente un, l'outil renvoie `captcha_required` avec un lien vers une page du serveur où l'utilisateur concerné le résout ; l'appel peut ensuite être relancé. Ce relais fonctionne pour un captcha simple (image, question). Pour un captcha lié au navigateur (reCAPTCHA, hCaptcha, Turnstile), le relais n'est pas fiable : l'utilisateur se connecte alors dans son propre navigateur et importe le cookie de session. La phase 0 dit lequel des deux cas s'applique.

**Isolation.** Chaque appel d'outil est rattaché à l'utilisateur authentifié. Magasin de cookies, cache privé, quotas et journal d'audit sont cloisonnés par utilisateur. Un test automatisé doit prouver qu'un utilisateur ne peut ni lire ni modifier les données d'un autre.

**Hors périmètre par défaut.** Suppression du compte Parfumo, changement de mot de passe ou d'adresse e-mail, et tout achat ou abonnement payant ne sont pas exposés comme outils.

## 7. Catalogue des outils

Le serveur expose environ 45 outils répartis en sept domaines. Cette liste est une hypothèse de travail à confirmer en phase 0 : les fonctions exactes du site priment.

Règle de dimensionnement : rester sous 50 outils. Au-delà, les assistants choisissent moins bien. Regrouper les variantes par un paramètre (`action`, `collection`) plutôt que multiplier les outils.

Type : L = lecture, E = écriture privée, P = écriture visible par d'autres membres, D = suppression.

### 7.1 Base de données (sans compte requis)

| Outil | Rôle | Type |
| --- | --- | --- |
| `parfumo_search_perfumes` | Recherche par texte et filtres : marque, parfumeur, notes, année, genre, concentration | L |
| `parfumo_get_perfume` | Fiche complète : pyramide de notes, accords, parfumeurs, année, notes moyennes (senteur, tenue, sillage, flacon, rapport qualité-prix), classification par saison et occasion | L |
| `parfumo_get_perfume_reviews` | Avis longs d'un parfum, paginés et triables | L |
| `parfumo_get_perfume_statements` | Avis courts (statements) d'un parfum | L |
| `parfumo_get_similar_perfumes` | Parfums similaires et dupes signalés | L |
| `parfumo_get_brand` | Fiche marque et liste de ses parfums | L |
| `parfumo_get_perfumer` | Fiche parfumeur et créations | L |
| `parfumo_get_note` | Fiche d'une note olfactive et parfums associés | L |
| `parfumo_get_rankings` | Classements : top, nouveautés, tendances, par année ou par genre | L |
| `parfumo_fetch_page` | Secours : lit une page quelconque de Parfumo et renvoie son texte nettoyé. GET uniquement, domaine Parfumo uniquement | L |

### 7.2 Collection

| Outil | Rôle | Type |
| --- | --- | --- |
| `parfumo_list_collections` | Listes de l'utilisateur avec effectifs : J'ai, J'ai eu, Wish List, Watch List, Testés, Décants, Miniatures, listes personnalisées | L |
| `parfumo_get_collection` | Contenu d'une liste, avec champs par flacon | L |
| `parfumo_add_to_collection` | Ajoute un parfum à une liste | E |
| `parfumo_move_collection_item` | Déplace un parfum d'une liste à une autre | E |
| `parfumo_update_collection_item` | Modifie les champs d'un flacon : contenance, niveau, prix, date d'achat, note privée, colonnes personnalisées | E |
| `parfumo_remove_from_collection` | Retire un parfum d'une liste | D |
| `parfumo_manage_custom_collection` | Crée, renomme ou supprime une liste personnalisée | E / D |
| `parfumo_export_collection` | Export complet en CSV ou JSON | L |
| `parfumo_collection_stats` | Statistiques calculées par le serveur : marques, notes dominantes, valeur, parfums jamais portés | L |

### 7.3 Ports (parfum du jour)

| Outil | Rôle | Type |
| --- | --- | --- |
| `parfumo_log_wear` | Enregistre le parfum porté, date optionnelle | E ou P selon le site |
| `parfumo_list_wears` | Historique des ports, filtrable par période | L |
| `parfumo_delete_wear` | Supprime un port | D |

### 7.4 Notes et avis

| Outil | Rôle | Type |
| --- | --- | --- |
| `parfumo_rate_perfume` | Note un parfum sur chaque échelle proposée par le site | P |
| `parfumo_classify_perfume` | Vote de classification : saison, occasion, type, public | P |
| `parfumo_list_my_ratings` | Notes déjà données par l'utilisateur | L |
| `parfumo_write_review` | Publie ou modifie un avis long | P |
| `parfumo_write_statement` | Publie ou modifie un avis court | P |
| `parfumo_delete_review_or_statement` | Supprime un avis | D |
| `parfumo_list_my_reviews` | Avis et statements de l'utilisateur | L |
| `parfumo_upload_perfume_photo` | Ajoute une photo à une fiche ou à son album | P |

### 7.5 Social

| Outil | Rôle | Type |
| --- | --- | --- |
| `parfumo_get_user` | Profil public d'un membre, ses listes et avis visibles | L |
| `parfumo_list_messages` | Boîte de réception et conversations privées | L |
| `parfumo_read_message_thread` | Contenu d'une conversation | L |
| `parfumo_send_message` | Envoie un message privé | P |
| `parfumo_get_notifications` | Notifications et fil d'activité | L |
| `parfumo_manage_follow` | Suit ou cesse de suivre un membre ; liste des abonnements | L / P |
| `parfumo_wall` | Lit un mur ; écrit sur le mur d'un membre | L / P |
| `parfumo_forum_browse` | Catégories, fils récents, recherche dans le forum | L |
| `parfumo_forum_read_thread` | Messages d'un fil, paginés | L |
| `parfumo_forum_post` | Ouvre un fil ou répond dans un fil | P |
| `parfumo_blog` | Lit les blogs ; publie ou modifie un billet | L / P |
| `parfumo_souk_browse` | Offres d'échange et de vente du Souk, recherche par parfum | L |
| `parfumo_souk_manage_offer` | Crée, modifie ou retire une offre de l'utilisateur | P / D |

### 7.6 Profil et compte

| Outil | Rôle | Type |
| --- | --- | --- |
| `parfumo_get_my_profile` | Profil, points, statistiques du compte | L |
| `parfumo_update_profile` | Texte de présentation, pays, avatar, parfums favoris | P |
| `parfumo_get_settings` | Réglages de notification et de confidentialité | L |
| `parfumo_update_settings` | Modifie ces réglages | E |
| `parfumo_session_status` | État de la liaison au compte Parfumo, quotas restants | L |

### 7.7 Contributions à la base (version 2)

Proposer un nouveau parfum ou une correction via Parfumo Research mobilise des bénévoles qui vérifient chaque soumission. À traiter après la version 1, avec un plafond quotidien strict.

### 7.8 Prompts et ressources MCP

Optionnels en version 1. Deux prompts utiles : recommandation à partir de la collection, et choix du parfum du jour selon la météo et l'historique des ports.

## 8. Conventions communes des outils

Tous les outils suivent les mêmes règles, pour qu'un assistant puisse les enchaîner sans surprise.

- **Nommage.** Préfixe `parfumo_`, verbe puis objet, en anglais et en `snake_case`.
- **Descriptions.** En anglais, une à trois phrases : ce que fait l'outil, quand l'utiliser, ce qu'il renvoie. Elles précisent si l'effet est visible par d'autres membres.
- **Schémas.** Entrées et sorties décrites en JSON Schema généré depuis les types Rust. Sortie structurée (`structuredContent`) accompagnée d'un texte court lisible.
- **Identifiants.** Chaque parfum, marque ou membre est renvoyé avec un identifiant stable et son URL Parfumo. Les outils d'écriture acceptent l'identifiant ou l'URL.
- **Pagination.** Paramètres `limit` (25 par défaut, 100 au plus) et `cursor`. Jamais de réponse de plus de 25 000 caractères environ : tronquer et renvoyer un curseur.
- **Annotations.** `readOnlyHint`, `destructiveHint`, `idempotentHint` et `openWorldHint` renseignés honnêtement sur chaque outil. Elles laissent chaque client décider d'afficher ou non une validation.
- **Essai à blanc.** Tout outil de type E, P ou D accepte `dry_run` (faux par défaut) et renvoie alors ce qui serait envoyé, sans rien modifier.
- **Retour d'écriture.** Après une écriture, l'outil relit la page concernée et renvoie l'état réel constaté, pas une simple supposition de succès.
- **Idempotence.** Ajouter un parfum déjà présent ou retirer un parfum absent n'est pas une erreur : l'outil renvoie l'état courant.
- **Contenu tiers.** Les textes rédigés par d'autres membres (avis, messages, forum) sont renvoyés comme données, délimités et marqués comme non fiables. Un message reçu peut contenir des instructions destinées à détourner l'assistant.

Erreurs normalisées, renvoyées avec `isError` et un message expliquant quoi faire :

| Code | Signification |
| --- | --- |
| `not_linked` | Aucun compte Parfumo relié pour cet utilisateur |
| `reauth_required` | Reconnexion impossible : identifiants à ressaisir |
| `captcha_required` | Le site demande une vérification humaine ; la réponse contient le lien pour la résoudre |
| `rate_limited` | Quota du serveur ou du site atteint ; délai d'attente indiqué |
| `not_found` | Parfum, membre ou élément introuvable |
| `parser_outdated` | La page a changé de structure : l'outil doit être mis à jour |
| `upstream_error` | Erreur ou indisponibilité de Parfumo |

## 9. Exigences non fonctionnelles

| Domaine | Exigence |
| --- | --- |
| Débit vers Parfumo | 1 requête par seconde au plus par utilisateur, 2 au total, avec délai aléatoire. Valeurs configurables |
| Plafonds d'écriture publique | Par utilisateur et par jour, configurables. Valeurs initiales proposées : 20 messages privés, 10 posts de forum, 10 avis ou statements |
| Reprise sur erreur | Attente exponentielle sur les codes 429 et 5xx ; arrêt immédiat sur 403 répété ou captcha |
| Cache | Fiches de la base : 24 h. Classements et recherches : 1 h. Données de compte : pas de cache, ou invalidation à chaque écriture |
| Journal d'audit | Chaque écriture enregistrée : utilisateur, outil, cible, horodatage, résultat. Consultable en ligne de commande. Conservation 90 jours |
| Secrets | Jamais dans les journaux, les erreurs ni les réponses d'outil. Test automatisé qui le vérifie |
| Sécurité HTTP | HTTPS obligatoire, validation de l'en-tête `Origin`, jetons à durée limitée, limitation des tentatives de connexion |
| Empreinte | Image Docker sous 50 Mo et mémoire sous 100 Mo au repos, hors conteneur navigateur |
| Performance | Surcoût du serveur inférieur à 50 ms par appel, hors temps de réponse de Parfumo |
| Observabilité | `/healthz`, journaux JSON, compteurs par outil : appels, erreurs, durée |
| Maintenance | Test canari quotidien sur un compte de test : il signale toute route ou page dont la structure a changé |
| Identification | En-tête `User-Agent` fixe et honnête, sans imitation tournante de navigateurs |

## 10. Tests et critères d'acceptation

La version 1 est acceptée quand les sept critères suivants sont remplis.

- [ ] Chaque outil de la liste validée en phase 0 fonctionne de bout en bout sur un compte Parfumo de test.
- [ ] Tests unitaires d'analyse sur fixtures HTML ou JSON enregistrées, sans accès réseau, exécutés en intégration continue.
- [ ] Conformité au protocole vérifiée avec MCP Inspector, puis avec au moins deux clients MCP différents.
- [ ] Test d'isolation : deux utilisateurs, aucune fuite de données ni d'action de l'un vers l'autre.
- [ ] Test de reconnexion : session Parfumo expirée, l'appel suivant réussit sans intervention.
- [ ] Aucun secret dans les journaux ni dans les réponses, vérifié par test.
- [ ] `docker compose up` sur un VPS vierge donne un serveur opérationnel en suivant le seul `README`.

Les tests d'écriture publique (avis, forum, messages) se font uniquement entre comptes de test, ou en `dry_run`, pour ne pas polluer la communauté.

## 11. Jalons et livrables

Six jalons, chacun livrable et testable seul. Le jalon 0 doit être validé par le commanditaire avant la suite.

1. **Jalon 0 : exploration.** `docs/DISCOVERY.md`, méthode d'accès retenue, liste finale des outils. Point d'arrêt.
2. **Jalon 1 : socle.** Serveur Streamable HTTP, authentification OAuth et jeton statique, gestion des utilisateurs, enrôlement du compte Parfumo, connexion et renouvellement de session, outils de la base de données (7.1).
3. **Jalon 2 : collection et ports.** Outils 7.2 et 7.3, export, statistiques.
4. **Jalon 3 : notes et avis.** Outils 7.4.
5. **Jalon 4 : social, profil et compte.** Outils 7.5 et 7.6, plafonds d'écriture publique.
6. **Jalon 5 : durcissement.** Audit, test canari, image Docker, `docker-compose.yml`, documentation.

Livrables finaux :

- Dépôt Git avec le code, les tests et l'intégration continue.
- `README.md` : installation, configuration, ajout d'un utilisateur, branchement d'un client MCP.
- `docs/DISCOVERY.md` et `docs/TOOLS.md` (référence de chaque outil avec exemples).
- `docs/MAINTENANCE.md` : que faire quand le site change, comment recapturer les fixtures.
- Image Docker et `docker-compose.yml`.

## 12. Points ouverts

Aucun point ouvert à ce stade. Le rapport d'exploration dira si une fonction exige un navigateur et quel type de captcha le site utilise.

Tranché le 11 novembre 2026 :

- Exclusions de la section 6 confirmées.
- Domaine cible : parfumo.com.
- Accès par requêtes uniquement, interfaces internes en priorité ; pas de navigateur en version 1.
- Captcha relayé à l'utilisateur, jamais résolu automatiquement.
- Liaison du compte par identifiants ou par cookie, au choix de chaque utilisateur.

## Sources

- [rmcp, SDK Rust officiel pour MCP](https://docs.rs/crate/rmcp) : fonctions et transports disponibles.
- [Spécification MCP 2026-07-28, annonce](https://blog.modelcontextprotocol.io/posts/2026-07-28/) et [journal des changements](https://modelcontextprotocol.io/specification/2026-07-28/changelog.md).
- [ShelfControl, annonce sur le forum Parfumo](https://www.parfumo.com/forums/post/237337) : vue tableur de la collection, export CSV, colonnes personnalisées.
- [Parfumo Research, base de connaissances](https://www.parfumo.com/research/knowledge-base/17) : fonctionnement des contributions.

Ces pages ont été consultées par extraits de résultats de recherche. Le catalogue d'outils s'appuie aussi sur les intitulés visibles des pages publiques de Parfumo ; il reste à confirmer sur le site connecté.
