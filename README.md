# Cleanup Hearts

Petite page web pour présenter une action de ramassage des déchets et permettre aux visiteurs d'envoyer un cœur de soutien.

## Stack

- Frontend Vite + TypeScript, sans framework.
- Backend Rust Axum + SQLx + SQLite embarqué.
- Déploiement en 2 containers : `frontend` public et `backend` privé.
- SQLite persisté dans un volume monté sur `/data`.

## Lancer avec Docker ou Podman

```bash
docker compose up --build
```

Ou selon l'environnement :

```bash
podman compose up --build
podman-compose up --build
```

La page est ensuite disponible sur :

```text
http://localhost:8080
```

Le backend n'est pas publié sur l'hôte. Le frontend sert les fichiers statiques et proxifie `/api/*` vers le service `backend` sur le réseau interne Compose.

## Développement local

Backend :

```bash
cd backend
DATABASE_URL=sqlite:data/hearts.db COOKIE_SECURE=false cargo run
```

Frontend :

```bash
cd frontend
npm install
npm run dev
```

Le serveur Vite proxifie `/api` vers `http://127.0.0.1:3000`.

## Configuration backend

Variables d'environnement disponibles :

```text
DATABASE_URL=sqlite:/data/hearts.db
BIND_ADDR=0.0.0.0:3000
COOKIE_NAME=cleanup_heart_vote
COOKIE_SECURE=true
COOKIE_SAME_SITE=Lax
VOTE_WINDOW_SECONDS=172800
```

En local HTTP, `COOKIE_SECURE=false` est nécessaire pour que le navigateur conserve le cookie. En production HTTPS, utiliser `COOKIE_SECURE=true`.

## API

```http
GET /api/hearts
```

```json
{
  "count": 123,
  "already_voted": false
}
```

```http
POST /api/hearts
```

Succès :

```json
{
  "count": 124,
  "voted": true
}
```

Déjà voté :

```json
{
  "count": 124,
  "voted": false,
  "reason": "already_voted",
  "retry_after_seconds": 172800
}
```

## Photos futures

Le frontend contient déjà la section :

```html
<section id="photos">
  <h2>Les photos de la journée</h2>
</section>
```

Elle peut recevoir plus tard une galerie responsive ou un carrousel simple avec des images statiques.
