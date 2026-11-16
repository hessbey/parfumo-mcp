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
| Cache : 24 h, 1 h, rien pour le compte | Abandonné à la version 0.2.3 | À la demande du commanditaire, plus aucune page n'est conservée : chaque lecture interroge le site, pour que la réponse ne soit jamais une copie datée |
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

## 2 quater. Changement postérieur à l'audit : l'interface de l'application (version 0.3.0)

Depuis la version 0.3.0, quand un compte Parfumo est relié avec son identifiant (pseudo ou
e-mail) et son mot de passe, une partie des lectures de la base de données passe par
l'interface JSON de l'application de Parfumo (`/app/v1`, décrite dans
[APP_API.md](APP_API.md), en anglais), avec le jeton de session de ce membre. Un compte relié
par cookie, ou l'absence de compte, n'y envoie rien : les pages web répondent, comme avant.
Le programme détient donc un secret de plus par utilisateur, et envoie le mot de passe à une
seconde adresse de connexion du même site. Ce changement a été relu en interne avant sa
publication, et les défauts trouvés ont été corrigés ; il n'a pas fait l'objet d'un nouvel
audit complet, ni d'une relecture par un tiers. Ce que ce changement oblige à tenir, dans la
forme de la section 2 :

| Exigence | État | Preuve |
| --- | --- | --- |
| Jeton stocké chiffré, par utilisateur | Tenu | Le jeton est rangé avec les cookies de session, dans le même bloc AES-256-GCM lié à l'utilisateur (colonne `cookies` de `parfumo_links`). Délier le compte, ou en relier un autre, l'efface de la base et de la mémoire. `a_linked_account_gets_keeps_and_renews_its_session_of_the_application_interface`, `unlinking_or_linking_another_account_takes_the_token_away_from_every_client` |
| Jeton jamais dans les journaux, le journal d'audit, les résultats d'outils, les erreurs, le rapport du canari | Tenu | Journaux, journal d'audit et résultats d'outils : `two_linked_users_never_carry_each_other_s_session_token`, `a_database_tool_reads_through_the_session_of_the_linked_account_and_the_web_site_otherwise`. Rapport du canari : `the_interface_area_reads_the_application_interface_with_the_token_of_the_canary_account`. Le jeton n'a pas de forme affichable (`AppSession`, `app.rs`) et la bibliothèque cliente n'écrit aucun journal |
| Aucune requête vers `/app/v1` sans jeton, sauf la connexion | Tenu | Refusée dans le transport avant tout envoi (`check_route` et `clearance`, `transport.rs`), par où sortent toutes les requêtes des deux binaires ; `parfumo_fetch_page` refuse toute adresse sous `/app/`. L'outil de développement `explore`, absent des binaires publiés, a sa propre sortie et applique la même règle. `nothing_goes_to_the_application_interface_without_the_token_except_the_login`, `without_a_token_nothing_is_sent_and_a_refused_token_is_never_sent_again`, `the_page_reader_sends_nothing_to_the_application_interface` |
| Un jeton refusé (401) n'est jamais renvoyé | Tenu | Oublié aussitôt pour tous les clients de l'utilisateur, et dans la base à la fin de l'appel d'outil (même abandonné en cours) : il ne revient pas après un redémarrage ; un appel qui attendait derrière le refus ne le renvoie pas. Un autre processus sur la même base ne le renvoie pas non plus : il reprend la session enregistrée au début de son appel suivant, et après la vérification de la session web (colonne `app_rev`), et n'écrit la sienne que sur celle qu'il a vue en dernier. Reste un appel déjà commencé dans l'autre processus, qui peut l'envoyer une fois. `calls_made_together_send_a_refused_token_once_and_log_in_once`, `a_call_that_starts_while_another_falls_back_does_not_bring_the_refused_token_back`, `a_token_the_site_refused_does_not_come_back_from_the_database`, `a_token_refused_to_one_process_is_not_sent_again_by_another_on_the_same_database`, `a_late_refusal_of_the_old_token_does_not_erase_the_session_another_process_renewed`, `a_refusal_met_by_one_process_is_stored_and_holds_for_the_other`, `an_account_tool_that_waited_for_the_web_site_does_not_send_a_token_refused_meanwhile`, `a_login_that_answers_as_a_call_ends_and_takes_the_stored_session_keeps_its_token` |
| Des identifiants refusés par l'interface ne lui sont pas renvoyés | Tenu | Colonne `app_refused` du lien, remise à zéro quand le compte est relié. Toute autre panne de la connexion est suivie d'une pause de 15 minutes. Une session renouvelée puis reperdue dans les 15 minutes n'est pas redemandée avant la fin de ce délai, redémarrage compris et entre deux processus sur la même base (colonnes `app_ended` et `app_renewed_at`). `credentials_the_interface_refused_are_never_sent_to_it_again_until_the_account_is_linked_again`, `an_outage_of_the_login_to_the_interface_is_followed_by_a_pause_not_taken_for_a_refusal`, `a_restart_does_not_forget_that_a_renewed_session_was_lost` |
| Le jeton n'est jamais envoyé à une page web, ni comme cookie | Tenu | Le transport refuse un appel fait au nom du membre hors de `/app/v1`, et le jeton n'entre pas dans les cookies envoyés. `nothing_goes_to_the_application_interface_without_the_token_except_the_login`, `login_keeps_a_token_that_is_sent_as_a_bearer_and_never_as_a_cookie`, `the_three_web_panels_are_read_on_the_web_site_by_a_client_without_token`. Sur le vrai site, en lecture seule, le 15 novembre 2026 : aucune requête vers les pages web ne portait le jeton |
| La clé propre à l'application n'est jamais utilisée | Tenu | Elle ne figure pas dans le code : la seule authentification posée sur une requête vers `/app/v1` est le jeton de session du membre (`send_once`, `transport.rs`). Aucun en-tête propre à l'application n'est envoyé ; le `User-Agent` reste celui de `parfumo-mcp` |
| Un jeton n'appartient qu'à un utilisateur et qu'à un compte | Tenu | Un seul détenteur par utilisateur, partagé par ses clients et par aucun autre ; un lien qui change pendant une connexion ne garde aucun jeton du compte précédent. `two_linked_users_never_carry_each_other_s_session_token`, `the_clients_of_one_user_hold_one_session`, `a_login_under_way_when_another_account_is_linked_leaves_nothing_in_the_new_link`, `a_link_that_changes_at_the_instant_a_login_leaves_keeps_nothing_of_the_previous_account` |
| Arrêt quand le site bloque l'adresse | Tenu | Dès que la page de blocage est rencontrée, plus rien n'est envoyé à `/app/v1` pendant cinq minutes, pour tous les utilisateurs du processus, et les lectures passent par les pages web. `the_block_page_stops_every_call_to_the_interface_for_five_minutes`, `while_the_address_is_blocked_every_read_goes_to_the_web_site_and_nothing_to_the_interface` |
| Débit : 1 requête par seconde par utilisateur, pages web et interface confondues | Tenu | Le client public et le client du compte d'un utilisateur partagent le même créneau de requête (`UserLimiter`, `transport.rs`) |

Les tests cités tournent sans réseau, contre un faux site. Ce qui n'a pas été mesuré sur le
vrai site est dans la section 3.

## 2 ter. Changement postérieur à l'audit : la page de connexion du binaire local

Depuis la version 0.2, le binaire local `parfumo-mcp` ouvre une page de connexion sur
l'ordinateur de l'utilisateur, au lieu de lire le compte dans la configuration du client MCP.
Cette page n'a pas été relue par un tiers. Ce qui la protège, et le test qui le vérifie
(`crates/parfumo-mcp/tests/stdio.rs`, `the_login_page_links_an_account_without_any_setting`) :

- Elle n'écoute que sur `127.0.0.1`, sur un port choisi par le système, et seulement dix
  minutes ou jusqu'à ce qu'un compte soit relié. Rien n'écoute le reste du temps.
- Son adresse contient un secret de 256 bits, renouvelé à chaque ouverture et comparé en
  temps constant. Toute autre adresse reçoit une réponse 404 vide.
- Elle ne répond qu'à son propre nom (`127.0.0.1:<port>` ou `localhost:<port>`) : un nom de
  domaine qu'un site ferait pointer vers l'ordinateur (réattribution DNS) est refusé.
- Un formulaire envoyé depuis un autre site (en-tête `Origin` étranger) est refusé, et rien
  n'est transmis à Parfumo.
- Mêmes en-têtes que les pages du serveur HTTP : aucun script, aucune ressource externe,
  aucune mise en cadre, aucune mise en cache.
- Le secret de l'adresse n'est pas journalisé. Le mot de passe et le cookie de session ne
  figurent ni sur la sortie standard, ni dans les journaux, ni en clair sur le disque.

Limite connue : l'adresse passe par la conversation, puisque c'est l'assistant qui la donne.
Elle ne mène qu'au formulaire : elle ne permet pas de lire ce qui est stocké. Celui qui la
connaît et peut joindre `127.0.0.1` sur cet ordinateur — donc un programme déjà présent sur
la machine — pourrait, pendant ces dix minutes, relier un autre compte ou délier le compte.
Un tel programme peut de toute façon lire les fichiers locaux de l'utilisateur.

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
- **Interface de l'application : la réponse à un mauvais mot de passe n'a jamais été
  enregistrée.** Le code ne tient pour un refus des identifiants qu'une réponse 401 en JSON,
  ou une réponse 2xx dont le JSON dit `"status":"error"`. Toute autre réponse de
  `/app/v1/auth/login` (400, 403, 404, 422, panne) est suivie d'une pause de 15 minutes, puis
  d'un nouvel essai au prochain appel d'outil. Si le site répond autrement à un mauvais mot
  de passe, les mêmes identifiants lui seraient renvoyés, au plus une fois par quart d'heure
  et par processus : cette pause est en mémoire, un redémarrage la remet à zéro.
- **Interface de l'application : la durée de vie d'un jeton de session est inconnue.** Le
  jeton reste stocké, chiffré, tant que le site ne l'a pas refusé et que le compte n'est pas
  délié. Délier le compte l'efface de la base et de la mémoire sans fermer la session auprès
  de Parfumo : le jeton y reste valable jusqu'à une échéance qu'on ne connaît pas.
- **Interface de l'application : la page de blocage de l'adresse n'a jamais été enregistrée
  comme fixture.** Elle est reconnue d'après ce qui en a été noté (statut 404, page HTML,
  « Access Denied … try again in … »). Une page de blocage d'une autre forme ne serait pas
  reconnue, et l'interface continuerait d'être interrogée.
