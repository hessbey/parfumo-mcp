# Maintenance — exploiter le serveur et le réparer quand Parfumo change

Ce document dit quoi faire dans quatre situations : le canari signale un changement du
site, un outil renvoie `parser_outdated`, Parfumo bloque le serveur, ou il faut exploiter
le serveur au quotidien (journal d'audit, sauvegarde, mise à jour).

Parfumo ne publie aucune API. Le serveur lit les pages et les appels internes du site : ils
changent sans préavis. Tout ce qui dépend de leur forme est dans un seul fichier,
`crates/parfumo-client/src/site.rs`.

## 1. Le test canari

Une fois par jour, le serveur lit sur le vrai site chaque type de page et de route dont les
outils dépendent, et vérifie que les analyseurs y trouvent ce qu'ils attendent.

| Sujet | Valeur |
| --- | --- |
| Heure | `PARFUMO_MCP_CANARY_HOUR`, heure UTC de 0 à 23. Défaut : 4. `off` coupe le canari |
| Pages publiques | 27 contrôles : recherche, fiche, avis, statements, similaires, marque, parfumeur, note, classements, page libre, forum, communautés, blogs, Souk, profil, mur, avis et liste d'un membre |
| Compte de test | 21 contrôles de plus si `PARFUMO_MCP_CANARY_USER` nomme un utilisateur du serveur dont le compte Parfumo est relié : listes, export CSV, ports, notes, avis, classification, messagerie, notifications, abonnements, réglages, profil |
| Écritures | Aucune. Le canari ne fait que lire, et n'utilise pas le cache |
| Débit | Celui des outils : une requête par seconde. Une soixantaine de requêtes ; 68 secondes mesurées le 12 novembre 2026 |
| Arrêt | Immédiat sur un captcha ou une limitation de débit : le reste est noté `skipped` |
| Alerte | `PARFUMO_MCP_CANARY_WEBHOOK` : une requête POST en JSON quand le résultat n'est pas `ok`, et une autre quand il redevient `ok`. Le corps porte la même phrase sous `text` et `content` (formats de Slack et de Discord) et la liste des contrôles en cause |
| Trace | Table `canary_runs` (90 jours), journal du serveur (niveau `ERROR` par contrôle en cause), et `"canary"` dans `/healthz` |

Utilisez un compte Parfumo de test pour `PARFUMO_MCP_CANARY_USER`, pas un compte personnel :
créez un utilisateur du serveur (`user add canari`), reliez-le au compte de test sur `/link`,
puis mettez `PARFUMO_MCP_CANARY_USER=canari` dans `.env`.

```sh
docker compose exec server parfumo-mcp-server canary --last     # dernier rapport enregistré
docker compose exec server parfumo-mcp-server canary            # un passage tout de suite
docker compose exec server parfumo-mcp-server canary --public-only --json
```

La commande sort avec le code 0 si tout est `ok`, 1 sinon. Lancée à la main pendant que le
serveur tourne, elle a son propre limiteur : elle ajoute au plus une requête par seconde à
celles du serveur. Ne la lancez pas en boucle.

### Lire un rapport

```
canary changed: 46 ok, 1 changed, 0 failed, 1 skipped (public pages and the Parfumo account of `canari`)
CHANGED  public   perfume      290 ms  parser outdated: rating block  [get_perfume]
skipped  public   perfumer       0 ms  needs the perfumer of the `perfume` check  [get_perfumer]
```

| Résultat d'un contrôle | Sens | Que faire |
| --- | --- | --- |
| `ok` | La route répond et l'analyseur trouve ce qu'il attend | Rien |
| `CHANGED` | La structure a changé : `parser outdated`, page introuvable, ou page lue mais vide de ce qu'elle doit contenir (`parsed, but …`) | Section 2 |
| `FAILED` | Pas de verdict : Parfumo injoignable, captcha, limitation, session du compte refusée | Section 3 |
| `skipped` | Non lancé : il dépend d'un contrôle qui n'a pas passé, ou le passage a été arrêté | Réparer d'abord le contrôle dont il dépend |

Statut du passage : `changed` dès qu'un contrôle est `CHANGED` ; sinon `inconclusive` s'il
reste des `FAILED` ou des `skipped` ; sinon `ok`. Le nom entre crochets est la fonction de
`parfumo-client` derrière le contrôle : c'est le point d'entrée de la réparation.

Un canari `ok` ne prouve pas tout. Il ne voit ni les écritures, ni les états que le compte
de test n'a pas (une notification, une conversation, une liste remplie).

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
   Si la forme des données a changé, `parse.rs` ou `parse/social.rs` suit.
4. **Remplacer la fixture** dans `crates/parfumo-client/tests/fixtures/` par la nouvelle
   page (section suivante), puis adapter le test qui la lit (`tests/parse_fixtures.rs`,
   `forum_fixtures.rs`, `market_fixtures.rs`, `social_fixtures.rs`).
5. **Vérifier** : `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D
   warnings`, `cargo fmt --all --check`, puis `parfumo-mcp-server canary` contre le vrai site.
6. **Noter le changement** dans `docs/DISCOVERY.md` (une ligne : ce qui était écrit, ce qui
   est mesuré) et déployer (section 5).

Si une fonction du site a disparu, l'outil doit le dire : retirez-le de la liste ou faites-lui
renvoyer une erreur claire, plutôt que de le laisser renvoyer du vide.

### Recapturer une fixture

Une fixture est la copie d'une réponse réelle. Les tests n'ont jamais accès au réseau.

| Type | Capture | Nettoyage obligatoire avant de l'ajouter au dépôt |
| --- | --- | --- |
| Page publique | `explore req GET <chemin> -o <nom>` sans s'être connecté (ou `curl` avec le User-Agent du projet) | Remplacer le `csrf_key` anonyme (16 caractères) par une valeur factice ; pseudos des membres → `member001`, `member002`… (partout : liens, textes, attributs) ; photos de profil (`user_photo/…`) → `00/00_0123456789abcdef…` ; identifiants `u_id` → `2000001`, `2000002`… |
| Page ou fragment connecté (`auth_*`) | `explore login` puis `explore req …` | Pseudo du compte → `testuser` ; pseudo d'un autre membre → `otheruser` ; identifiants numériques de membres → `1000001`, `1000002` ; tout jeton de 32 caractères hexadécimaux ou plus → `0123456789abcdef…` ; adresse e-mail et pays retirés |
| Réponse d'API (`auth_api_*.json`) | `explore req POST /api/… k=v` | Même règle |

Une page servie à une session porte toujours le pseudo du compte et le `csrf_key` de la
session, même si elle paraît publique : ne l'enregistrez jamais telle quelle. Le test
`auth_fixtures_are_scrubbed` échoue si une fixture `auth_*` contient un jeton réel ; il ne
détecte pas un pseudo oublié.

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
| `upstream_error` répété | Parfumo en panne, ou 403 | `canary --public-only` dit si le site répond encore au serveur |

Le serveur ne contourne rien : pas de rotation d'adresse, pas de résolution de captcha, pas
d'imitation de navigateur. Son `User-Agent` est fixe et dit ce qu'il est.

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
cookies enregistrés sont illisibles et chaque utilisateur doit relier son compte à nouveau.

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
git pull
docker compose build
docker compose up -d
curl https://<votre-domaine>/healthz
scripts/check-image.sh        # facultatif : taille de l'image, démarrage, mémoire au repos
```

Le schéma de la base se met à jour seul au démarrage. Les sessions Parfumo et les jetons
OAuth survivent au redémarrage.

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
verdict du canari : `{"canary":{"at":…,"status":"ok"},"status":"ok","version":"0.1.0"}`. Le
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
   git tag v0.1.0
   git push origin v0.1.0
   ```

   Le tag doit correspondre exactement à la version de `Cargo.toml`, sinon la publication
   s'arrête avant de rien produire.
3. En une dizaine de minutes, la publication :
   - compile les deux binaires pour Linux (Intel/AMD et ARM), macOS (Apple silicon et Intel)
     et Windows, et vérifie qu'ils démarrent ;
   - crée la page de version sur GitHub, avec les archives et le fichier `SHA256SUMS` ;
   - publie l'image Docker du serveur pour amd64 et arm64 sur
     `ghcr.io/hessbey/parfumo-mcp-server`, sous le numéro de version et sous `latest`.

Un tag de pré-version (`v0.2.0-rc.1`) crée une pré-version et ne déplace pas `latest`.

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
