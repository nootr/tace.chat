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

        // Check if this is an unknown contact
        const isUnknown = contact.name.startsWith('Unknown (');

        contactEl.innerHTML = `
            <img src="img/logo.png" alt="User" class="profile-pic">
            <div class="contact-details">
                <div class="contact-name">${contact.name}</div>
                <div class="last-message">${getLastMessage(contact.id)}</div>
            </div>
            ${isUnknown ? '<span class="unknown-badge">New</span>' : ''}
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
    const isUnknown = contact.name.startsWith('Unknown (');

    chatHeader.innerHTML = `
        <img src="img/logo.png" alt="Profile" class="profile-pic">
        <div class="chat-info">
            <div class="chat-name">${contact.name}</div>
            ${isUnknown ? `<div class="chat-status">Public key: ${contact.publicKey.substring(0, 16)}...</div>` : ''}
        </div>
        ${isUnknown ? `
        <div class="header-icons">
            <button onclick="renameContact('${contact.id}')" title="Rename Contact">
                <svg viewBox="0 0 24 24" width="20" height="20">
                    <path fill="currentColor" d="M20.71,7.04C21.1,6.65 21.1,6 20.71,5.63L18.37,3.29C18,2.9 17.35,2.9 16.96,3.29L15.12,5.12L18.87,8.87M3,17.25V21H6.75L17.81,9.93L14.06,6.18L3,17.25Z"></path>
                </svg>
            </button>
        </div>` : ''}
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

function renameContact(contactId) {
    const contact = state.contacts.find(c => c.id === contactId);
    if (!contact) return;

    const newName = prompt(`Enter a new name for this contact:\n\nPublic key: ${contact.publicKey}`, contact.name);
    if (newName && newName.trim()) {
        contact.name = newName.trim();
        saveState();
        renderContactList();
        renderChatWindow();
    }
}

// Make renameContact available globally for onclick
window.renameContact = renameContact;

async function sendMessage() {
    const text = messageInput.value.trim();
    if (!text || !state.activeContactId) return;

    const contact = state.contacts.find(c => c.id === state.activeContactId);
    if (!contact) return;

    try {
        // 1. Encrypt the message
        const encrypted = encrypt(state.keys.private_key, contact.publicKey, text);

        // 2. Prepare the message payload for the API
        const messagePayload = {
            to: contact.publicKey,
            from: state.keys.public_key, // Include sender's public key
            // Convert Uint8Array to a regular array for JSON serialization
            ciphertext: Array.from(encrypted.ciphertext),
            nonce: Array.from(encrypted.nonce),
        };

        // 3. Send the message to the backend
        const response = await fetch('http://localhost:3001/message', {
            method: 'POST',
            headers: {
                'Content-Type': 'application/json',
            },
            body: JSON.stringify(messagePayload),
        });

        if (!response.ok) {
            const errorData = await response.json();
            console.error('Failed to send message:', errorData);
            alert(`Failed to send message: ${errorData.error || 'Unknown error'}`);
            return;
        }

        // 4. If sending is successful, save to local state and update UI
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
        console.error("Encryption or network error:", e);
        alert("Failed to send message. Could not encrypt or send.");
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

// --- Message Polling ---
let pollingInterval = null;

async function pollMessages() {
    if (!state.keys || !state.keys.public_key) return;

    try {
        const response = await fetch(`http://localhost:3001/poll?public_key=${encodeURIComponent(state.keys.public_key)}`, {
            method: 'GET',
            headers: {
                'Content-Type': 'application/json',
            },
        });

        if (!response.ok) {
            console.error('Failed to poll messages:', response.status);
            return;
        }

        const data = await response.json();
        if (data.messages && data.messages.length > 0) {
            // Process each message
            for (const msg of data.messages) {
                // Find the contact who sent this message by matching public key
                let contact = state.contacts.find(c => c.publicKey === msg.from);

                if (!contact) {
                    // Create a new contact for unknown sender
                    console.log('Received message from unknown contact:', msg.from);

                    // Try to decrypt first to make sure we can read the message
                    try {
                        const decryptedText = decrypt(
                            state.keys.private_key,
                            msg.from,
                            new Uint8Array(msg.ciphertext),
                            new Uint8Array(msg.nonce)
                        );

                        // If decryption succeeded, create the contact
                        contact = {
                            id: `contact_${Date.now()}_${Math.random()}`,
                            name: `Unknown (${msg.from.substring(0, 8)}...)`,
                            publicKey: msg.from
                        };

                        state.contacts.push(contact);
                        console.log('Added unknown contact:', contact.name);
                    } catch (e) {
                        console.error('Failed to decrypt message from unknown sender:', msg.from, e);
                        continue;
                    }
                }

                try {
                    // Decrypt using the sender's public key
                    const decryptedText = decrypt(
                        state.keys.private_key,
                        contact.publicKey,
                        new Uint8Array(msg.ciphertext),
                        new Uint8Array(msg.nonce)
                    );

                    // Create message object
                    const message = {
                        id: Date.now() + Math.random(), // Ensure unique ID
                        ciphertext: new Uint8Array(msg.ciphertext),
                        nonce: new Uint8Array(msg.nonce),
                        timestamp: new Date(msg.timestamp * 1000).toISOString(),
                        sender: 'them',
                    };

                    if (!state.messages[contact.id]) {
                        state.messages[contact.id] = [];
                    }

                    // Check if message already exists (avoid duplicates)
                    const exists = state.messages[contact.id].some(m =>
                        m.timestamp === message.timestamp &&
                        m.sender === 'them'
                    );

                    if (!exists) {
                        state.messages[contact.id].push(message);

                        // Update UI if this contact is active
                        if (state.activeContactId === contact.id) {
                            renderChatWindow();
                        }
                        renderContactList();
                        saveState();

                        console.log('Received message from', contact.name);
                    }
                } catch (e) {
                    console.error('Failed to decrypt message from', contact.name, e);
                }
            }
        }
    } catch (e) {
        console.error('Error polling messages:', e);
    }
}

function startPolling() {
    // Poll immediately
    pollMessages();

    // Then poll every 5 seconds
    pollingInterval = setInterval(pollMessages, 5000);
}

function stopPolling() {
    if (pollingInterval) {
        clearInterval(pollingInterval);
        pollingInterval = null;
    }
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
    startPolling(); // Start polling for messages
    console.log('App Initialized with encryption. State:', state);
}

main();
