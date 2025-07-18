import init, { generate_keypair, encrypt, decrypt, sign } from './pkg/tace_webclient.js';

const bootstrapNode = '__BOOTSTRAP_NODE_URL__';
const apiProtocol = '__API_PROTOCOL__';

const collectorUrl = '__COLLECTOR_URL__';

function hexToUint8Array(hexString) {
    if (hexString.length % 2 !== 0) {
        throw "Invalid hexString";
    }
    const arrayBuffer = new Uint8Array(hexString.length / 2);
    for (let i = 0; i < hexString.length; i += 2) {
        const byteValue = parseInt(hexString.substr(i, 2), 16);
        if (isNaN(byteValue)) {
            throw "Invalid hexString";
        }
        arrayBuffer[i / 2] = byteValue;
    }
    return arrayBuffer;
}

document.addEventListener('alpine:init', () => {
    Alpine.data('app', function () {
        return {
            keys: this.$persist({ private_key: '', public_key: '' }).as('tace_keys'),
            contacts: this.$persist([]).as('tace_contacts'),
            messages: this.$persist({}).as('tace_messages'),
            activeContactId: null,
            node: bootstrapNode,

            settingsModalOpen: false,
            addContactModalOpen: false,
            metricsModalOpen: false,
            showTermsModal: this.$persist(true).as('tace_terms_modal_open'),
            termsAccepted: false,
            search: '',
            newMessage: '',
            newContact: { name: '', publicKey: '' },
            isPolling: false,
            metricsLoaded: false,

            init() {
                this.$watch('showTermsModal', async (val) => {
                    if (!val) {
                        await this.initWasmAndApp();
                    }
                });

                if (!this.showTermsModal) {
                    this.initWasmAndApp();
                }

                this.$watch('activeContactId', () => {
                    this.$nextTick(this.scrollToBottom);
                });

                this.$watch('metricsModalOpen', (val) => {
                    if (val) {
                        this.$nextTick(() => {
                            if (!this.metricsLoaded) {
                                this.fetchMetrics();
                                this.metricsLoaded = true;
                            }
                        });
                    } else {
                        if (this.metricsChart) {
                            this.metricsChart.destroy();
                            this.metricsChart = null;
                        }
                        this.metricsLoaded = false;
                    }
                });
            },

            async initWasmAndApp() {
                await init();
                await this.loadNodeUrl();
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
            },

            // Computed Properties
            get filteredContacts() {
                const searchLower = this.search.toLowerCase();
                const filtered = this.search
                    ? this.contacts.filter(c => c.name.toLowerCase().includes(searchLower))
                    : [...this.contacts];

                return filtered.sort((a, b) => {
                    const lastMsgA = this.messages[a.id]?.slice(-1)[0];
                    const lastMsgB = this.messages[b.id]?.slice(-1)[0];

                    const timeA = lastMsgA ? new Date(lastMsgA.timestamp).getTime() : 0;
                    const timeB = lastMsgB ? new Date(lastMsgB.timestamp).getTime() : 0;

                    return timeB - timeA;
                });
            },

            get activeContact() {
                return this.contacts.find(c => c.id === this.activeContactId);
            },

            // Methods
            getUrl(path = '', node = null) {
                node = node || this.node;
                return `${apiProtocol}://${node}${path}`;
            },

            getPublicKeyColor(publicKey) {
                if (!publicKey) {
                    return '#ccc'; // Default color for placeholders
                }
                // Simple hash function to get a color from the public key
                let hash = 0;
                for (let i = 0; i < publicKey.length; i++) {
                    hash = publicKey.charCodeAt(i) + ((hash << 5) - hash);
                }
                const c = (hash & 0x00FFFFFF).toString(16).toUpperCase();
                return "#" + "00000".substring(0, 6 - c.length) + c;
            },

            async apiRequest(path, options = {}, retries = 1) {
                const url = this.getUrl(path);
                try {
                    const response = await fetch(url, options);
                    if (!response.ok) {
                        // For 4xx errors, don't retry, just throw
                        if (response.status >= 400 && response.status < 500) {
                            const errData = await response.json().catch(() => ({ error: 'Request failed with client error.' }));
                            throw new Error(errData.error || `HTTP error! status: ${response.status}`);
                        }
                        throw new Error(`HTTP error! status: ${response.status}`);
                    }
                    // Handle cases with no content
                    const text = await response.text();
                    return text ? JSON.parse(text) : {};
                } catch (error) {
                    console.error(`API request to ${url} failed:`, error);
                    if (retries > 0) {
                        console.log('Attempting to fetch a new node and retry...');
                        await this.fetchNewNode();
                        return this.apiRequest(path, options, retries - 1);
                    } else {
                        console.error('API request failed after multiple retries.');
                        throw error; // Re-throw the error after retries are exhausted
                    }
                }
            },

            async fetchNewNode() {
                console.log('Fetching a new node from bootstrap:', bootstrapNode);
                try {
                    const response = await fetch(this.getUrl('/connect', bootstrapNode));
                    const data = await response.json();
                    if (data && data.node) {
                        this.node = data.node;
                        console.log('Switched to new node:', this.node);
                    }
                } catch (err) {
                    console.error('Failed to fetch a new node from bootstrap:', err);
                    // If bootstrap fails, we keep the current node and let the retry logic handle it.
                }
            },

            async loadNodeUrl() {
                try {
                    const data = await this.apiRequest('/connect');
                    if (data && data.node) {
                        this.node = data.node;
                        console.log('Connected to node:', this.node);
                    }
                } catch (err) {
                    console.error('Error fetching node URL:', err);
                }
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

                    await this.apiRequest('/message', {
                        method: 'POST',
                        headers: { 'Content-Type': 'application/json' },
                        body: JSON.stringify(payload),
                    });

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
                    this.$nextTick(() => this.scrollToBottom());
                } catch (e) {
                    console.error("Failed to send message:", e);
                    alert("Failed to send message. The node might be offline.");
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

            getLastMessageTime(contactId) {
                const contactMessages = this.messages[contactId];
                if (!contactMessages || contactMessages.length === 0) {
                    return 'No messages yet...';
                }
                const lastMsg = contactMessages[contactMessages.length - 1];
                const msgDate = new Date(lastMsg.timestamp);
                const now = new Date();

                if (msgDate.toDateString() === now.toDateString()) {
                    return msgDate.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
                } else {
                    // Check if it was yesterday
                    const yesterday = new Date(now);
                    yesterday.setDate(now.getDate() - 1);
                    if (msgDate.toDateString() === yesterday.toDateString()) {
                        return 'Yesterday';
                    }
                    // Otherwise, show the date
                    return msgDate.toLocaleDateString();
                }
            },

            copyToClipboard(element) {
                navigator.clipboard.writeText(element.value);
            },

            attemptCloseTermsModal() {
                if (this.termsAccepted) {
                    this.showTermsModal = false;
                }
            },

            scrollToBottom() {
                const chatWindow = document.getElementById('chat-window');
                if (chatWindow) {
                    chatWindow.scrollTop = chatWindow.scrollHeight;
                }
            },

            async fetchMetrics() {
                try {
                    console.log(`Fetching metrics from: ${collectorUrl}/metrics`);
                    const response = await fetch(`${collectorUrl}/metrics`);
                    if (!response.ok) {
                        throw new Error(`HTTP error! status: ${response.status}`);
                    }
                    const data = await response.json();
                    console.log('Fetched metrics data:', data);
                    this.metricsData = data;
                    this.renderChart();
                } catch (e) {
                    console.error("Failed to fetch metrics:", e);
                    // Optionally, display an error message in the modal
                }
            },

            renderChart() {
                if (!this.metricsData || this.metricsData.length === 0) {
                    console.log('No metrics data to render or data is empty.', this.metricsData);
                    if (this.metricsChart) {
                        this.metricsChart.destroy();
                        this.metricsChart = null;
                    }
                    return;
                }

                const reversedMetricsData = [...this.metricsData].reverse();
                const ctx = this.$refs.metricsCanvas.getContext('2d');

                if (this.metricsChart) {
                    this.metricsChart.destroy();
                }

                const labels = reversedMetricsData.map(m => new Date(m.timestamp * 1000).toLocaleTimeString());
                const nodeCountData = reversedMetricsData.map(m => m.network_size_estimate);
                const totalMessagesData = reversedMetricsData.map(m => m.total_network_keys_estimate);

                this.metricsChart = new Chart(ctx, {
                    type: 'line',
                    data: {
                        labels: labels,
                        datasets: [
                            {
                                label: 'Node Count',
                                data: nodeCountData,
                                borderColor: 'rgb(75, 192, 192)',
                                tension: 0.1,
                                fill: false
                            },
                            {
                                label: 'Total Messages',
                                data: totalMessagesData,
                                borderColor: 'rgb(255, 99, 132)',
                                tension: 0.1,
                                fill: false
                            }
                        ]
                    },
                    options: {
                        responsive: true,
                        maintainAspectRatio: false,
                        scales: {
                            y: {
                                beginAtZero: true
                            }
                        }
                    }
                });
            },

            startPolling() {
                this.pollMessages();
                setInterval(() => this.pollMessages(), 5000);
            },

            async pollMessages() {
                if (!this.keys.public_key || this.isPolling) return;

                this.isPolling = true;
                try {
                    // 1. Get challenge
                    const challengeData = await this.apiRequest(`/poll/challenge?public_key=${encodeURIComponent(this.keys.public_key)}`);
                    const nonceHex = challengeData.nonce;
                    if (!nonceHex) {
                        throw new Error("Did not receive a challenge nonce.");
                    }
                    const nonce = hexToUint8Array(nonceHex);

                    // 2. Sign challenge
                    const signature = sign(this.keys.private_key, nonce);

                    // 3. Poll with signed challenge
                    const payload = {
                        public_key: this.keys.public_key,
                        nonce: Array.from(nonce),
                        signature: Array.from(signature),
                    };

                    const data = await this.apiRequest('/poll', {
                        method: 'POST',
                        headers: { 'Content-Type': 'application/json' },
                        body: JSON.stringify(payload),
                    });


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
                                this.$nextTick(() => this.scrollToBottom());
                            }
                        });
                    }
                } catch (e) {
                    console.error('Error polling messages:', e.message);
                } finally {
                    this.isPolling = false;
                }
            }
        }
    });
});
