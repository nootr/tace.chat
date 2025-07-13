import init, { generate_keypair, encrypt, decrypt } from '../webclient/pkg/tace_webclient.js';

const bootstrapNode = 'node1:3001';

document.addEventListener('alpine:init', () => {
    Alpine.data('app', function () {
        return {
            // State
            keys: this.$persist({ private_key: '', public_key: '' }).as('tace_keys'),
            contacts: this.$persist([]).as('tace_contacts'),
            messages: this.$persist({}).as('tace_messages'),
            activeContactId: null,
            node: bootstrapNode,

            // UI State
            settingsModalOpen: false,
            addContactModalOpen: false,
            search: '',
            newMessage: '',
            newContact: { name: '', publicKey: '' },

            // Init
            async init() {
                await init(); // Initialize WASM
                this.loadNodeUrl();
                if (!this.keys.private_key || !this.keys.public_key) {
                    console.log('No keys found, generating new ones...');
                    const keypair = generate_keypair();
                    this.keys = {
                        private_key: keypair.private_key,
                        public_key: keypair.public_key,
                    };
                    keypair.free();
                }
                this.startPolling();
                console.log('App Initialized');

                this.$watch('activeContactId', () => {
                    this.$nextTick(() => {
                        const chatWindow = document.getElementById('chat-window');
                        if (chatWindow) {
                            chatWindow.scrollTop = chatWindow.scrollHeight;
                        }
                    });
                });
            },

            // Computed Properties
            get filteredContacts() {
                if (!this.search) return this.contacts;
                return this.contacts.filter(contact =>
                    contact.name.toLowerCase().includes(this.search.toLowerCase())
                );
            },

            get activeContact() {
                return this.contacts.find(c => c.id === this.activeContactId);
            },

            // Methods
            loadNodeUrl() {
                fetch(`http://${this.node}/connect`)
                    .then(res => res.json())
                    .then(data => {
                        if (data && data.node) {
                            this.node = data.node;
                            console.log('Connected to node:', this.node);
                        }
                    })
                    .catch(err => console.error('Error fetching node URL:', err));
            },

            selectContact(contactId) {
                this.activeContactId = contactId;
            },

            async sendMessage() {
                if (!this.newMessage.trim() || !this.activeContactId) return;

                const contact = this.activeContact;
                try {
                    const encrypted = encrypt(this.keys.private_key, contact.publicKey, this.newMessage);
                    const payload = {
                        to: contact.publicKey,
                        from: this.keys.public_key,
                        ciphertext: Array.from(encrypted.ciphertext),
                        nonce: Array.from(encrypted.nonce),
                    };

                    const response = await fetch(`http://${this.node}/message`, {
                        method: 'POST',
                        headers: { 'Content-Type': 'application/json' },
                        body: JSON.stringify(payload),
                    });

                    if (!response.ok) throw new Error('Failed to send message');

                    const message = {
                        id: Date.now(),
                        ciphertext: encrypted.ciphertext,
                        nonce: encrypted.nonce,
                        timestamp: new Date().toISOString(),
                        sender: 'me',
                    };

                    if (!this.messages[this.activeContactId]) {
                        this.messages[this.activeContactId] = [];
                    }
                    this.messages[this.activeContactId].push(message);
                    this.newMessage = '';
                } catch (e) {
                    console.error("Failed to send message:", e);
                    alert("Failed to send message.");
                }
            },

            addContact() {
                const name = this.newContact.name.trim();
                const publicKey = this.newContact.publicKey.trim();
                const publicKeyRegex = /^([0-9a-fA-F]{66}|[0-9a-fA-F]{130})$/;

                if (name && publicKey && publicKeyRegex.test(publicKey)) {
                    this.contacts.push({ id: `contact_${Date.now()}`, name, publicKey });
                    this.newContact = { name: '', publicKey: '' };
                    this.addContactModalOpen = false;
                } else {
                    alert('Invalid contact details. Please provide a name and a valid public key.');
                }
            },

            renameContact(contactId) {
                const contact = this.contacts.find(c => c.id === contactId);
                if (!contact) return;
                const newName = prompt(`Enter a new name for this contact:`, contact.name);
                if (newName && newName.trim()) {
                    contact.name = newName.trim();
                }
            },

            decryptMessage(msg) {
                try {
                    const contact = this.contacts.find(c => c.publicKey === msg.from || (this.activeContact && c.id === this.activeContactId));
                    if (!contact) return '[Decryption Error: Contact not found]';

                    const myPrivateKey = this.keys.private_key;
                    const theirPublicKey = contact.publicKey;

                    const ciphertext = msg.ciphertext instanceof Uint8Array ? msg.ciphertext : new Uint8Array(Object.values(msg.ciphertext));
                    const nonce = msg.nonce instanceof Uint8Array ? msg.nonce : new Uint8Array(Object.values(msg.nonce));

                    return decrypt(myPrivateKey, theirPublicKey, ciphertext, nonce);
                } catch (e) {
                    console.error("Decryption failed:", e);
                    return '[Could not decrypt message]';
                }
            },

            getLastMessage(contactId) {
                const contactMessages = this.messages[contactId];
                if (!contactMessages || contactMessages.length === 0) return 'No messages yet...';
                return 'Encrypted message';
            },

            copyToClipboard(element) {
                navigator.clipboard.writeText(element.value);
            },

            startPolling() {
                this.pollMessages();
                setInterval(() => this.pollMessages(), 5000);
            },

            async pollMessages() {
                if (!this.keys.public_key) return;
                try {
                    const response = await fetch(`http://${this.node}/poll?public_key=${encodeURIComponent(this.keys.public_key)}`);
                    if (!response.ok) return;
                    const data = await response.json();

                    if (data.messages && data.messages.length > 0) {
                        data.messages.forEach(msg => {
                            let contact = this.contacts.find(c => c.publicKey === msg.from);
                            if (!contact) {
                                try {
                                    decrypt(this.keys.private_key, msg.from, new Uint8Array(msg.ciphertext), new Uint8Array(msg.nonce));
                                    contact = {
                                        id: `contact_${Date.now()}_${Math.random()}`,
                                        name: `Unknown (${msg.from.substring(0, 8)}...)`,
                                        publicKey: msg.from
                                    };
                                    this.contacts.push(contact);
                                } catch (e) {
                                    console.error('Failed to decrypt message from unknown sender:', msg.from, e);
                                    return;
                                }
                            }

                            if (!this.messages[contact.id]) {
                                this.messages[contact.id] = [];
                            }

                            const messageExists = this.messages[contact.id].some(m =>
                                m.timestamp === new Date(msg.timestamp * 1000).toISOString() && m.sender === 'them'
                            );

                            if (!messageExists) {
                                const newMsg = {
                                    id: Date.now() + Math.random(),
                                    ciphertext: new Uint8Array(msg.ciphertext),
                                    nonce: new Uint8Array(msg.nonce),
                                    timestamp: new Date(msg.timestamp * 1000).toISOString(),
                                    sender: 'them',
                                    from: msg.from,
                                };
                                this.messages[contact.id].push(newMsg);
                            }
                        });
                    }
                } catch (e) {
                    console.error('Error polling messages:', e);
                }
            }
        }
    });
});
