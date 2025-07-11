import init, { generate_keypair, encrypt, decrypt } from '../webclient/pkg/tace_webclient.js';

// --- DOM Elements ---
const contactList = document.getElementById('contact-list');
const chatHeader = document.getElementById('chat-header');
const chatWindow = document.getElementById('chat-window');
const messageInput = document.getElementById('message-input');
const sendBtn = document.getElementById('send-btn');
const noChatSelected = document.querySelector('.no-chat-selected');

// Modals & Buttons
const settingsModal = document.getElementById('settings-modal');
const addContactModal = document.getElementById('add-contact-modal');
const userPublicKeyText = document.getElementById('user-public-key');
const addContactForm = document.getElementById('add-contact-form');

// --- State ---
let state = {
    keys: null, // { private_key, public_key }
    contacts: [], // { id, name, publicKey }
    messages: {}, // { contactId: [ { id, ciphertext, nonce, timestamp, sender } ] }
    activeContactId: null,
};

// --- Local Storage ---
const STORAGE_KEY = 'taceChatState';

function saveState() {
    try {
        localStorage.setItem(STORAGE_KEY, JSON.stringify(state));
    } catch (error) {
        console.error("Failed to save state:", error);
    }
}

function loadState() {
    const savedState = localStorage.getItem(STORAGE_KEY);
    if (savedState) {
        let parsedState;
        try {
            parsedState = JSON.parse(savedState);
        } catch (e) {
            console.error("Could not parse state from localStorage. Starting fresh.", e);
            // If parsing fails, reset the state.
            localStorage.removeItem(STORAGE_KEY);
            return;
        }

        const publicKeyRegex = /^([0-9a-fA-F]{66}|[0-9a-fA-F]{130})$/;
        const privateKeyRegex = /^[0-9a-fA-F]{64}$/;

        // Validate own keys
        if (parsedState.keys && (!publicKeyRegex.test(parsedState.keys.public_key) || !privateKeyRegex.test(parsedState.keys.private_key))) {
            console.warn("Invalid format for own keys found in localStorage. Clearing keys to trigger regeneration.");
            parsedState.keys = null;
        }

        // Validate contact public keys and filter out invalid ones
        if (parsedState.contacts) {
            const originalCount = parsedState.contacts.length;
            parsedState.contacts = parsedState.contacts.filter(contact => {
                if (contact && contact.publicKey && publicKeyRegex.test(contact.publicKey)) {
                    return true;
                }
                const contactName = contact ? contact.name : 'unknown contact';
                console.warn(`Removing contact "${contactName}" due to invalid public key format.`);
                if (contact && parsedState.messages && parsedState.messages[contact.id]) {
                    delete parsedState.messages[contact.id];
                }
                return false;
            });
            if (parsedState.contacts.length < originalCount) {
                alert('Some contacts were removed due to invalid or corrupted public key data.');
            }
        }

        // Message ciphertexts are stored as arrays, need to convert them to Uint8Array
        if (parsedState.messages) {
            for (const contactId in parsedState.messages) {
                if (!Array.isArray(parsedState.messages[contactId])) continue;
                parsedState.messages[contactId].forEach(msg => {
                    if (msg.ciphertext && typeof msg.ciphertext !== 'string') {
                        msg.ciphertext = new Uint8Array(Object.values(msg.ciphertext));
                    }
                    if (msg.nonce && typeof msg.nonce !== 'string') {
                        msg.nonce = new Uint8Array(Object.values(msg.nonce));
                    }
                });
            }
        }

        // Only assign to state if parsedState is not null
        if(parsedState) {
            state = { ...state, ...parsedState };
        }
    }
}

// --- UI Rendering ---
function renderContactList() {
    contactList.innerHTML = '';
    if (state.contacts.length === 0) {
        contactList.innerHTML = '<p style="text-align: center; color: #8e8e8e; padding: 20px;">No contacts yet. Click "+" to add one.</p>';
        return;
    }
    state.contacts.forEach(contact => {
        const contactEl = document.createElement('div');
        contactEl.className = `contact ${contact.id === state.activeContactId ? 'active' : ''}`;
        contactEl.dataset.contactId = contact.id;
        contactEl.innerHTML = `
            <img src="img/logo.png" alt="User" class="profile-pic">
            <div class="contact-details">
                <div class="contact-name">${contact.name}</div>
                <div class="last-message">${getLastMessage(contact.id)}</div>
            </div>
        `;
        contactEl.addEventListener('click', () => selectContact(contact.id));
        contactList.appendChild(contactEl);
    });
}

async function renderChatWindow() {
    const noChatSelectedEl = document.querySelector('.no-chat-selected');

    if (!state.activeContactId) {
        chatHeader.innerHTML = '';
        // Ensure the placeholder is visible and the chat window is cleared.
        chatWindow.innerHTML = '<div class="no-chat-selected" style="display: flex; flex-direction: column; justify-content: center; align-items: center; height: 100%; text-align: center; color: var(--text-secondary);"><img src="img/logo.png" alt="Tace Chat" class="logo-large"><h1>Tace Chat</h1><p>Select a chat to start messaging.</p></div>';
        messageInput.disabled = true;
        sendBtn.disabled = true;
        return;
    }

    // If we have an active chat, the no-chat-selected element inside the window is removed,
    // so we don't need to hide it.
    const contact = state.contacts.find(c => c.id === state.activeContactId);
    chatHeader.innerHTML = `
        <img src="img/logo.png" alt="Profile" class="profile-pic">
        <div class="chat-info">
            <div class="chat-name">${contact.name}</div>
        </div>
    `;

    chatWindow.innerHTML = '';
    const messages = state.messages[contact.id] || [];
    for (const msg of messages) {
        const msgEl = document.createElement('div');
        msgEl.className = `message ${msg.sender === 'me' ? 'sent' : 'received'}`;

        let decryptedText = '[Could not decrypt message]';
        try {
            // Decrypt using my private key and the contact's public key
            const myPrivateKey = state.keys.private_key;
            const theirPublicKey = contact.publicKey;

            decryptedText = decrypt(myPrivateKey, theirPublicKey, msg.ciphertext, msg.nonce);
        } catch (e) {
            console.error("Decryption failed:", e);
        }

        msgEl.innerHTML = `
            <div class="message-content">
                <p>${decryptedText}</p>
                <span class="timestamp">${new Date(msg.timestamp).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}</span>
            </div>
        `;
        chatWindow.appendChild(msgEl);
    }
    chatWindow.scrollTop = chatWindow.scrollHeight;
    messageInput.disabled = false;
    sendBtn.disabled = false;
    messageInput.focus();
}

function getLastMessage(contactId) {
    const messages = state.messages[contactId];
    if (!messages || messages.length === 0) {
        return 'No messages yet...';
    }
    // This is now too expensive to do for the whole list.
    // We will just show "Encrypted message"
    return 'Encrypted message';
}

// --- Event Handlers & Logic ---
function selectContact(contactId) {
    state.activeContactId = contactId;
    renderContactList();
    renderChatWindow();
}

function sendMessage() {
    const text = messageInput.value.trim();
    if (!text || !state.activeContactId) return;

    const contact = state.contacts.find(c => c.id === state.activeContactId);
    if (!contact) return;

    try {
        const encrypted = encrypt(state.keys.private_key, contact.publicKey, text);
        const message = {
            id: Date.now(),
            ciphertext: encrypted.ciphertext,
            nonce: encrypted.nonce,
            timestamp: new Date().toISOString(),
            sender: 'me',
        };

        if (!state.messages[state.activeContactId]) {
            state.messages[state.activeContactId] = [];
        }
        state.messages[state.activeContactId].push(message);

        saveState();
        renderChatWindow();
        renderContactList();
        messageInput.value = '';
    } catch (e) {
        console.error("Encryption failed:", e);
        alert("Failed to send message. Could not encrypt.");
    }
}

function setupEventListeners() {
    sendBtn.addEventListener('click', sendMessage);
    messageInput.addEventListener('keydown', (e) => {
        if (e.key === 'Enter') sendMessage();
    });

    document.getElementById('settings-btn').addEventListener('click', () => {
        userPublicKeyText.value = state.keys.public_key;
        settingsModal.style.display = 'flex';
    });

    document.getElementById('add-contact-btn').addEventListener('click', () => {
        addContactModal.style.display = 'flex';
    });

    document.querySelectorAll('.modal-container .close-btn').forEach(btn => {
        btn.addEventListener('click', () => {
            settingsModal.style.display = 'none';
            addContactModal.style.display = 'none';
        });
    });

    document.getElementById('copy-public-key-btn').addEventListener('click', () => {
        userPublicKeyText.select();
        document.execCommand('copy');
    });

    addContactForm.addEventListener('submit', (e) => {
        e.preventDefault();
        const name = document.getElementById('contact-name').value;
        const publicKeyInput = document.getElementById('contact-public-key').value.trim();

        // A public key can be 66 (compressed) or 130 (uncompressed) hex characters.
        const publicKeyRegex = /^([0-9a-fA-F]{66}|[0-9a-fA-F]{130})$/;

        if (name && publicKeyInput && publicKeyRegex.test(publicKeyInput)) {
            state.contacts.push({ id: `contact_${Date.now()}`, name, publicKey: publicKeyInput });
            saveState();
            renderContactList();
            addContactForm.reset();
            addContactModal.style.display = 'none';
        } else {
            alert('Invalid contact details. Please ensure the name is not empty and you have provided a valid public key (66 or 130 hex characters).');
        }
    });
}

// --- Initialization ---
async function main() {
    await init(); // Initialize the WASM module

    loadState();
    if (!state.keys) {
        console.log('No keys found, generating new ones via WASM...');
        const keypair = generate_keypair();
        state.keys = {
            private_key: keypair.private_key,
            public_key: keypair.public_key,
        };
        keypair.free();
        saveState();
    }
    renderContactList();
    renderChatWindow();
    setupEventListeners();
    console.log('App Initialized with encryption. State:', state);
}

main();
