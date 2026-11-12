# parfumo-mcp

[English](README.md) · **Français** · [Deutsch](README.de.md)

[![ci](https://github.com/hessbey/parfumo-mcp/actions/workflows/ci.yml/badge.svg)](https://github.com/hessbey/parfumo-mcp/actions/workflows/ci.yml)
[![licence : MIT](https://img.shields.io/badge/licence-MIT-blue.svg)](LICENSE)
![rust 1.90+](https://img.shields.io/badge/rust-1.90%2B-orange.svg)

Un serveur [Model Context Protocol](https://modelcontextprotocol.io), écrit en Rust, qui
donne à un assistant IA un accès à [Parfumo](https://www.parfumo.com) : la base de données
des parfums, votre collection, vos ports, vos notes et avis, et la partie communautaire
(messages, forum, blogs, Souk).

> **Non officiel.** Ce projet n'est ni affilié à Parfumo ni approuvé par Parfumo. Il lit les
> mêmes pages qu'un navigateur, à une requête par seconde. L'accès automatisé peut malgré
> tout conduire Parfumo à suspendre le compte relié : vous l'utilisez à vos risques.

## Ce que vous obtenez

- **48 outils**, tous préfixés `parfumo_`. Les 10 outils de la base de données et les
  lectures publiques (forum, blogs, Souk, profils) fonctionnent **sans compte Parfumo**.
- **Deux binaires, les mêmes outils :**

  | Binaire | Transport | Pour qui |
  | --- | --- | --- |
  | `parfumo-mcp` | stdio | Une personne, sur sa machine. Ni port, ni OAuth, rien à héberger |
  | `parfumo-mcp-server` | Streamable HTTP | Plusieurs personnes, comptes isolés, sur un serveur derrière HTTPS (image Docker) |

- **Prudent par défaut.** Chaque outil d'écriture accepte `dry_run` et relit l'état réel.
  Tout ce que d'autres membres peuvent voir reste en essai à blanc tant que vous n'activez
  pas les écritures publiques, puis est plafonné par outil et par jour.
- **Vérifiable.** Chaque appel d'outil est journalisé en local ; `parfumo-mcp audit --writes`
  montre ce qui a réellement été écrit sur votre compte.
- **Respectueux du site.** Une requête par seconde par utilisateur, délai aléatoire,
  `User-Agent` fixe et honnête, cache de 24 h pour les fiches et de 1 h pour les recherches
  et classements. Aucune résolution de captcha, aucun contournement de Cloudflare.
- **Vise `www.parfumo.com`, le site en anglais.** Le site allemand `www.parfumo.de` utilise
  d'autres adresses et d'autres libellés et n'est pas pris en charge.
- **Les secrets restent hors de la conversation.** Les identifiants viennent de
  l'environnement (stdio) ou d'une page web `/link` (HTTP), sont chiffrés au repos en
  AES-256-GCM et n'apparaissent jamais dans les journaux, les erreurs ou les résultats
  d'outils.

## Démarrage rapide (local)

Téléchargez l'archive correspondant à votre système depuis la
[dernière version](https://github.com/hessbey/parfumo-mcp/releases/latest) et décompressez-la. Elle contient le programme `parfumo-mcp`,
prêt à l'emploi sous Linux, macOS et Windows. Pour le compiler vous-même, voir
[Usage local](docs/LOCAL.md#install).

Ajoutez-le à Claude Code :

```sh
claude mcp add parfumo \
  -e PARFUMO_COOKIE='pf_auth=…' \
  -- /chemin/vers/parfumo-mcp
```

Ou à tout client configuré par un fichier JSON (Claude Desktop et la plupart des autres) :

```json
{
  "mcpServers": {
    "parfumo": {
      "command": "/chemin/vers/parfumo-mcp",
      "env": { "PARFUMO_COOKIE": "pf_auth=…" }
    }
  }
}
```

`PARFUMO_COOKIE` est le cookie `pf_auth` copié depuis un navigateur connecté à Parfumo. Vous
pouvez utiliser `PARFUMO_USER` et `PARFUMO_PASSWORD` à la place, ou rien du tout : sans
compte, les outils publics fonctionnent quand même. Détail : [Usage local](docs/LOCAL.md).

Pour l'héberger pour plusieurs personnes, voir [Hébergement](docs/HOSTING.md) : le serveur
est publié sous forme d'image Docker, `ghcr.io/hessbey/parfumo-mcp-server`.

## Les outils

| Domaine | Outils | Compte nécessaire |
| --- | --- | --- |
| Base de données | recherche, fiche parfum, avis, statements, parfums similaires, marque, parfumeur, note, classements, lecture d'une page | Non |
| Collection | lister, lire, ajouter, déplacer, modifier, retirer, listes personnalisées, export CSV, statistiques | Oui |
| Ports | enregistrer, lister, supprimer | Oui |
| Notes et avis | noter, classifier, écrire ou supprimer un avis ou un statement, lister les siens | Oui |
| Social | profils des membres, messages, notifications, abonnements, mur, forum, blogs, Souk, communautés | Lectures : le plus souvent non. Écritures : oui |
| Profil et compte | profil, réglages, état de la session | Oui |

Liste complète : [Catalogue des outils](docs/TOOL_CATALOG.md).

## Un mot sur les textes

Parfumo interdit les textes produits par une IA dans les avis, les statements, le forum et
les messages, et restreint ou supprime les comptes qui en publient. Les outils qui envoient
un texte le disent à l'assistant : il doit transmettre **vos** mots, pas les écrire à votre
place.

## Documentation

Les guides sont en anglais ; les références détaillées sont en français.

| Guide | Contenu |
| --- | --- |
| [Usage local](docs/LOCAL.md) | Le binaire stdio : compte, branchement d'un client, fichiers locaux |
| [Hébergement](docs/HOSTING.md) | Le serveur HTTP : Docker, utilisateurs, OAuth, liaison d'un compte Parfumo |
| [Configuration](docs/CONFIGURATION.md) | Toutes les variables d'environnement ; verrou, plafonds et destinataires des écritures publiques |
| [Catalogue des outils](docs/TOOL_CATALOG.md) | Les 48 outils, conventions, codes d'erreur |
| [État du projet](docs/STATUS.md) | Ce qui est prouvé sur le vrai site, ce qui ne l'est pas, ce qui reste à faire |
| [Référence des outils](docs/TOOLS.md) | En français : paramètres et sorties réelles de chaque outil |
| [Tous les documents](docs/README.md) | Index complet |

## Comment c'est construit

```
crates/parfumo-client      bibliothèque : accès au site. Transport, routes et sélecteurs
                           (un seul module, src/site.rs), analyse HTML, modèles
crates/parfumo-mcp-core    bibliothèque : les 48 outils, la session Parfumo, le stockage,
                           le journal d'audit
crates/parfumo-mcp         binaire local : les outils sur stdio
crates/parfumo-mcp-server  binaire hébergé : les outils sur HTTP, avec OAuth, utilisateurs,
                           canari
```

Parfumo est un site rendu côté serveur, sans API publique : le client analyse donc du HTML.
Toutes les routes et tous les sélecteurs sont dans un seul module, les analyseurs sont
testés sur des fixtures enregistrées, et un test canari quotidien en lecture seule, sur le
serveur hébergé, signale toute page dont la structure a changé.

## Développement

```sh
cargo test --workspace     # sans réseau : fixtures enregistrées et faux serveurs locaux
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
```

L'intégration continue lance les trois sous Linux, relance les tests sous macOS et Windows,
compile les deux binaires en `release`, puis construit l'image Docker et vérifie sa taille,
son démarrage et sa mémoire au repos. Pousser un tag `vX.Y.Z` publie une version : les
binaires pour cinq cibles et l'image Docker.

## Licence

[MIT](LICENSE)
