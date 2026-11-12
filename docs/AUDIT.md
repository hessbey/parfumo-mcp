# Audit du jalon 5 — durcissement

12 novembre 2026. Ce document rend compte de l'audit prévu par la section 11.6 du cahier des
charges : ce qui a été vérifié, ce qui a été corrigé, ce qui reste ouvert.

Méthode : relecture de sécurité indépendante du code exposé (OAuth, page `/link`,
authentification, chiffrement, sessions, transport vers Parfumo), puis vérification de
chaque constat dans le code, correction et test. La relecture n'a exécuté aucun code ; les
corrections sont couvertes par `cargo test` (188 tests, sans réseau) et, pour le canari et
le journal d'audit, par un essai sur le vrai site.

Aucune faille critique n'a été trouvée : pas d'accès anonyme à `/mcp`, pas de fuite d'un
utilisateur vers un autre, pas de SSRF dans le client Parfumo, pas de secret dans les
journaux relus.

## 1. Constats et corrections

| # | Gravité | Constat | Correction | Test |
| --- | --- | --- | --- | --- |
| 1 | Moyenne | **Une connexion refusée par Parfumo était rejouée à chaque appel d'outil** : sonde puis envoi des mêmes identifiants refusés, deux fois par appel, sans fin. Contraire à « arrêt immédiat sur 403 répété », et risque de blocage du compte | Un lien au statut `reauth_required` ou `expired` n'envoie plus rien, pas même la sonde, tant que l'utilisateur n'a pas relié son compte sur `/link`. Après un captcha ou trois connexions échouées sans refus (panne), pause de 15 minutes pour cet utilisateur | `credentials_mode_logs_in_again_and_gives_up_after_three_attempts`, `an_expired_session_is_renewed_and_the_call_replayed` |
| 2 | Moyenne | **Épuisement de ressources sans authentification** : chaque envoi du formulaire de connexion lance un calcul Argon2, le verrou ne portait que sur le nom ; `/oauth/register` était ouvert et ses lignes jamais purgées ; `GET /oauth/authorize?client_id=https://…` faisait aller chercher n'importe quelle URL publique, sans mémoire des échecs | Deux calculs Argon2 à la fois, pas plus. Verrou par adresse du client (20 échecs en 15 minutes, lue dans `X-Forwarded-For` posé par Caddy) en plus du verrou par nom (5 échecs). Enregistrement de clients : 30 par minute, 500 au plus, purge après 30 jours sans jeton. Documents de métadonnées : 10 téléchargements par minute, échec retenu 10 minutes | `one_address_cannot_try_many_keys`, `a_window_lets_through_its_quota_only`, `expired_access_token_is_rejected_and_refresh_is_not_a_bearer` |
| 3 | Basse | Les durées des jetons OAuth acceptaient n'importe quelle valeur | Bornées : accès de 1 minute à 1 jour, rafraîchissement de 1 heure à 90 jours | — |
| 4 | Basse | **Un seul verrou de reconnexion pour tous les utilisateurs**, tenu pendant les requêtes : un compte au cookie mort ralentissait les autres | Un verrou par utilisateur | Suite existante |
| 5 | Basse | `parfumo_fetch_page` suivait une redirection avant de la contrôler : une page de Parfumo qui redirige vers `/api/…` faisait demander cette route (par le client anonyme, sans cookie de compte) | Chaque redirection est contrôlée avant d'être suivie | `not_found_and_redirects` |
| 6 | Basse | La page de consentement OAuth affichait le nom déclaré par le client comme s'il était vérifié, et seulement le domaine de retour | Le nom est marqué « non vérifié », l'adresse de retour est affichée en entier, avec une consigne | `oauth_pkce_flow_end_to_end` |
| 7 | Basse | Rotation des jetons de rafraîchissement sans détection de rejeu : un jeton volé et utilisé le premier se renouvelait sans fin | Un jeton de rafraîchissement présenté deux fois révoque tous les jetons issus de la même autorisation | `oauth_pkce_flow_end_to_end`, `expired_access_token_is_rejected_and_refresh_is_not_a_bearer` |
| 8 | Basse | Le filtre d'adresses publiques laissait passer `192.0.0.0/24`, `198.18.0.0/15`, NAT64 et 6to4 | Plages ajoutées | `private_addresses_are_not_fetched` |
| 9 | Basse | **Le journal d'audit ne distinguait pas une écriture réelle d'un essai à blanc**, et une ligne perdue l'était en silence | Colonne `writes` : nombre de requêtes d'écriture réellement envoyées, comptées au seul point d'envoi (`Clients::send`). Une ligne qui ne s'écrit pas est signalée au niveau `ERROR`. Filtres et compteurs en ligne de commande | `collection_writes_are_idempotent_dry_runnable_and_read_back`, `cache_ttl_and_audit` |

## 2. Exigences de la section 9

| Exigence | État | Preuve |
| --- | --- | --- |
| Débit : 1 requête par seconde par utilisateur, 2 au total, délai aléatoire | Tenu | `config.rs` borne les valeurs (jamais moins de 1000 ms, jamais plus de 2 par seconde) ; limiteur commun aux outils et au canari |
| Plafonds d'écriture publique | Tenu | Jalon 4 : par outil, par utilisateur et par jour, persistants |
| Reprise sur erreur : attente sur 429 et 5xx, arrêt sur 403 répété ou captcha | Tenu depuis ce jalon | Constat 1 |
| Cache : 24 h, 1 h, rien pour le compte | Tenu | Le client du compte n'a pas de cache ; le canari non plus |
| Journal d'audit : chaque écriture, consultable, 90 jours | Tenu | Constat 9 ; `parfumo-mcp-server audit --writes` |
| Secrets jamais dans les journaux, les erreurs, les réponses | Tenu | Tests des jalons 1 à 4, plus le rapport du canari (`the_canary_reads_the_linked_account_and_never_writes`) ; journal du serveur et rapport du canari relus après l'essai réel : ni cookie ni jeton |
| HTTPS | Tenu par le déploiement | Caddy ; le serveur seul avertit si l'URL publique n'est pas `https` |
| Validation de `Origin` | Partiel | Une origine étrangère est refusée sur `/mcp` et sur les formulaires. Une requête **sans** en-tête `Origin` est acceptée : c'est le cas de tout client MCP qui n'est pas un navigateur |
| Jetons à durée limitée | Partiel | Jetons OAuth : oui (constat 3). La clé d'accès de l'utilisateur (jeton porteur statique) n'expire pas : c'est le repli demandé par la section 6 ; elle se remplace par `user rotate-key` |
| Limitation des tentatives de connexion | Tenu | Constat 2 |
| Image sous 50 Mo, mémoire sous 100 Mo au repos | Tenu | 20 Mo de mémoire résidente au repos (binaire `release`, 12 novembre 2026). Binaire de 10,8 Mo. Image : construite et contrôlée par l'intégration continue à chaque push (`scripts/check-image.sh` échoue au-delà de 50 Mo d'image ou de 100 Mo de mémoire au repos) |
| Surcoût inférieur à 50 ms par appel | Tenu | 2,7 ms en médiane, 11,9 ms au pire sur 30 appels d'un outil sans requête vers Parfumo |
| Observabilité : `/healthz`, journaux JSON, compteurs par outil | Tenu | `parfumo-mcp-server audit --stats` : appels, erreurs, écritures, durée moyenne et maximale par outil |
| Test canari quotidien sur un compte de test | Tenu | `canary.rs` ; 48 contrôles `ok` sur le vrai site le 12 novembre 2026 |
| `User-Agent` fixe et honnête | Tenu | `transport.rs` |

## 2 bis. Changement postérieur à l'audit : une clé d'accès par utilisateur

Après cet audit, le nom d'utilisateur et le mot de passe du serveur ont été supprimés.
Chaque utilisateur n'a plus qu'un secret, sa clé d'accès (`pmcp_…`, 256 bits aléatoires,
seule son empreinte SHA-256 est stockée). Elle sert de jeton porteur, s'utilise sur la page
`/link` et sur la page de connexion OAuth. Conséquences sur les constats ci-dessus :

- Constat 2 : il n'y a plus de calcul Argon2 ni de verrou par nom. Il reste le verrou par
  adresse (20 clés fausses en 15 minutes), testé par `one_address_cannot_try_many_keys`.
- Un jeton d'accès OAuth n'est pas accepté à la place de la clé sur les pages
  (`oauth_pkce_flow_end_to_end`).
- Remplacer la clé (`user rotate-key`) révoque les sessions OAuth de l'utilisateur.
- Les formulaires étaient refusés par tous les navigateurs (origine vide due à
  `Referrer-Policy: no-referrer`) : corrigé, testé par
  `forms_are_accepted_from_this_server_only_and_the_pages_keep_their_origin` et vérifié
  dans un vrai navigateur.

Ce changement n'a pas fait l'objet d'un nouvel audit complet.

## 3. Ce qui reste ouvert

- **`docker compose up` n'a jamais été lancé sur un VPS.** C'est le septième critère
  d'acceptation (section 10) et il n'est pas prouvé : `docker-compose.yml` et le `Caddyfile`
  sont relus, pas lancés. L'image elle-même est construite et contrôlée par l'intégration
  continue depuis la mise en ligne du dépôt (`scripts/check-image.sh` : taille sous 50 Mo,
  utilisateur non root, démarrage, mémoire au repos sous 100 Mo).
- **Le déclenchement quotidien du canari** n'a pas été observé : le calcul de l'heure est
  testé, le passage lui-même a été lancé à la main.
- **Limitation par adresse** : elle repose sur l'en-tête `X-Forwarded-For` de Caddy. Sans
  reverse proxy devant le serveur, cet en-tête est falsifiable et rien ne freine les essais.
- **Pas de plafond global** sur les connexions échouées : un attaquant réparti sur beaucoup
  d'adresses n'est arrêté que par la taille des clés d'accès, 256 bits tirés au hasard.
- **Comportement de Cloudflare depuis l'adresse du VPS** : inconnu, comme aux jalons précédents.
- **Deux clients MCP différents** (section 10) : MCP Inspector seulement ; aucun client
  OAuth réel n'a été essayé.
- La relecture n'a pas couvert en entier les fichiers d'outils (`*_tools.rs`) : l'absence
  d'appel aux routes de suppression de compte, de mot de passe et d'e-mail repose sur une
  recherche de ces noms dans le code.
