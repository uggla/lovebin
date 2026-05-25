import '@picocss/pico/css/pico.min.css';
import './style.css';

type HeartsResponse = {
  count: number;
  already_voted: boolean;
};

type VoteResponse = {
  count: number;
  voted: boolean;
  reason?: 'already_voted';
  retry_after_seconds?: number;
};

const countElement = mustQuery<HTMLSpanElement>('#heart-count');
const button = mustQuery<HTMLButtonElement>('#heart-button');
const message = mustQuery<HTMLParagraphElement>('#vote-message');

void loadHearts();

button.addEventListener('click', () => {
  void sendHeart();
});

async function loadHearts(): Promise<void> {
  setButtonState('loading');

  try {
    const response = await fetch('/api/hearts', {
      credentials: 'same-origin'
    });

    if (!response.ok) {
      throw new Error(`GET /api/hearts failed with ${response.status}`);
    }

    const data = (await response.json()) as HeartsResponse;
    updateCount(data.count);

    if (data.already_voted) {
      setButtonState('already-voted');
      message.textContent = 'Tu as déjà envoyé un cœur ❤️';
    } else {
      setButtonState('ready');
      message.textContent = 'Clique sur le cœur pour nous encourager !';
    }
  } catch (error) {
    console.error(error);
    setButtonState('ready');
    message.textContent = "Oups, le compteur n'a pas pu arriver. Réessaie plus tard.";
  }
}

async function sendHeart(): Promise<void> {
  setButtonState('sending');

  try {
    const response = await fetch('/api/hearts', {
      method: 'POST',
      credentials: 'same-origin'
    });

    if (!response.ok) {
      throw new Error(`POST /api/hearts failed with ${response.status}`);
    }

    const data = (await response.json()) as VoteResponse;
    updateCount(data.count);

    if (data.voted) {
      setButtonState('voted');
      message.textContent =
        'Merci ! Ton cœur aide à encourager les petits gestes pour la planète 🌍';
      playHeartAnimation();
      return;
    }

    setButtonState('already-voted');
    message.textContent = data.retry_after_seconds
      ? `Tu as déjà envoyé un cœur. Tu pourras recommencer dans ${formatRetryAfter(
          data.retry_after_seconds
        )}.`
      : 'Tu as déjà envoyé un cœur ❤️';
  } catch (error) {
    console.error(error);
    setButtonState('ready');
    message.textContent = "Oups, le cœur n'est pas parti. Réessaie plus tard.";
  }
}

function updateCount(count: number): void {
  countElement.textContent = new Intl.NumberFormat('fr-FR').format(count);
  countElement.closest('.heart-count')?.classList.remove('pop');
  window.setTimeout(() => countElement.closest('.heart-count')?.classList.add('pop'), 20);
}

function setButtonState(state: 'loading' | 'ready' | 'sending' | 'voted' | 'already-voted'): void {
  button.classList.toggle('is-sending', state === 'sending' || state === 'loading');

  if (state === 'loading') {
    button.disabled = true;
    button.innerHTML = '<span aria-hidden="true">♥</span> Chargement...';
    return;
  }

  if (state === 'sending') {
    button.disabled = true;
    button.innerHTML = '<span aria-hidden="true">♥</span> Envoi du cœur...';
    return;
  }

  if (state === 'voted') {
    button.disabled = true;
    button.innerHTML = '<span aria-hidden="true">♥</span> Merci pour ton cœur';
    return;
  }

  if (state === 'already-voted') {
    button.disabled = true;
    button.innerHTML = '<span aria-hidden="true">♥</span> Tu as déjà envoyé un cœur';
    return;
  }

  button.disabled = false;
  button.innerHTML = '<span aria-hidden="true">♥</span> J’envoie un cœur';
}

function playHeartAnimation(): void {
  const burst = document.createElement('span');
  burst.className = 'heart-burst';
  burst.setAttribute('aria-hidden', 'true');
  burst.textContent = '♥ ♥ ♥';
  button.append(burst);
  window.setTimeout(() => burst.remove(), 1200);
}

function formatRetryAfter(seconds: number): string {
  const hours = Math.ceil(seconds / 3600);

  if (hours < 24) {
    return `${hours} h`;
  }

  const days = Math.ceil(hours / 24);
  return `${days} jour${days > 1 ? 's' : ''}`;
}

function mustQuery<T extends Element>(selector: string): T {
  const element = document.querySelector<T>(selector);

  if (!element) {
    throw new Error(`Missing required page element: ${selector}`);
  }

  return element;
}
