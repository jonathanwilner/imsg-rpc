;;; imsg.el --- Emacs client for imsg RPC -*- lexical-binding: t; -*-

;; This file provides a lightweight JSON-RPC client for the `imsg rpc` command.

(require 'cl-lib)
(require 'json)
(require 'subr-x)
(require 'transient)
(require 'notifications nil t)

(defgroup imsg nil
  "Emacs client for the imsg JSON-RPC interface."
  :group 'applications)

(defcustom imsg-binary "imsg"
  "Path to the imsg executable."
  :type 'string
  :group 'imsg)

(defcustom imsg-db-path nil
  "Optional path to the Messages SQLite database."
  :type '(choice (const :tag "Default" nil) string)
  :group 'imsg)

(defcustom imsg-rpc-extra-args nil
  "Extra arguments to pass to `imsg rpc`."
  :type '(repeat string)
  :group 'imsg)

(defcustom imsg-request-timeout 5
  "Seconds to wait for a synchronous RPC response."
  :type 'number
  :group 'imsg)

(defcustom imsg-transport 'tramp
  "Transport used for RPC: local, tramp, or network."
  :type '(choice (const :tag "Local process" local)
                 (const :tag "TRAMP SSH" tramp)
                 (const :tag "Network socket" network))
  :group 'imsg)

(defcustom imsg-remote-method "ssh"
  "TRAMP method used for remote connections."
  :type 'string
  :group 'imsg)

(defcustom imsg-remote-host "192.168.2.186"
  "Default remote host for `imsg rpc`."
  :type 'string
  :group 'imsg)

(defcustom imsg-remote-user nil
  "Optional remote user for TRAMP connections."
  :type '(choice (const :tag "Default" nil) string)
  :group 'imsg)

(defcustom imsg-remote-directory nil
  "TRAMP directory for running `imsg rpc` remotely.
When nil, runs locally. Example: \"/ssh:user@mac-host:\"."
  :type '(choice (const :tag "Local" nil) string)
  :group 'imsg)

(defcustom imsg-network-host "127.0.0.1"
  "Host for the network transport."
  :type 'string
  :group 'imsg)

(defcustom imsg-network-port 57999
  "Port for the network transport."
  :type 'integer
  :group 'imsg)

(defcustom imsg-notify-enabled t
  "When non-nil, show Emacs notifications for incoming messages."
  :type 'boolean
  :group 'imsg)

(defcustom imsg-notify-function #'imsg--default-notify
  "Function called with a message alist to show a notification."
  :type 'function
  :group 'imsg)

(defvar imsg-on-message-hook nil
  "Hook run with a single argument (message alist) for watch notifications.")

(defvar imsg--process nil)
(defvar imsg--partial "")
(defvar imsg--next-id 0)
(defvar imsg--pending (make-hash-table :test 'equal))
(defvar imsg--subscriptions (make-hash-table :test 'equal))
(defvar imsg--subscription-tokens (make-hash-table :test 'equal))
(defvar imsg--desired-subscriptions (make-hash-table :test 'equal))
(defvar imsg--contact-cache (make-hash-table :test 'equal))
(defvar imsg--recipient-history nil)

(defface imsg-sent-face
  '((t :foreground "white" :background "DodgerBlue3"))
  "Face for sent message text."
  :group 'imsg)

(defface imsg-recv-face
  '((t :foreground "black" :background "gray90"))
  "Face for received message text."
  :group 'imsg)

(defface imsg-reply-face
  '((t :foreground "gray50"))
  "Face for reply indicators."
  :group 'imsg)

(defcustom imsg-auto-reconnect t
  "When non-nil, automatically restart the RPC process and resubscribe."
  :type 'boolean
  :group 'imsg)

(defun imsg--rpc-command ()
  "Return the command list used to launch `imsg rpc`."
  (append
   (list imsg-binary "rpc")
   (when imsg-db-path (list "--db" imsg-db-path))
   imsg-rpc-extra-args))

(defun imsg--start-local-process (buf)
  (make-process
   :name "imsg-rpc"
   :buffer buf
   :command (imsg--rpc-command)
   :connection-type 'pipe
   :coding 'utf-8-unix
   :noquery t
   :filter #'imsg--filter
   :sentinel #'imsg--sentinel))

(defun imsg--start-remote-process (buf)
  (let* ((default-directory imsg-remote-directory)
         (process-connection-type nil)
         (args (imsg--rpc-command))
         (proc (apply #'start-file-process "imsg-rpc" buf (car args) (cdr args))))
    (set-process-filter proc #'imsg--filter)
    (set-process-sentinel proc #'imsg--sentinel)
    (set-process-coding-system proc 'utf-8-unix 'utf-8-unix)
    (set-process-query-on-exit-flag proc nil)
    proc))

(defun imsg--start-network-process (buf)
  (let ((proc (make-network-process
               :name "imsg-rpc"
               :buffer buf
               :host imsg-network-host
               :service imsg-network-port
               :coding 'utf-8-unix
               :noquery t
               :filter #'imsg--filter
               :sentinel #'imsg--sentinel)))
    (set-process-query-on-exit-flag proc nil)
    proc))

(defun imsg--ensure-process ()
  "Ensure the imsg RPC process is running and return it."
  (unless (process-live-p imsg--process)
    (let* ((buf (get-buffer-create "*imsg-rpc*"))
           (proc (pcase imsg-transport
                   ('network (imsg--start-network-process buf))
                   ('local (imsg--start-local-process buf))
                   (_ (if imsg-remote-directory
                          (imsg--start-remote-process buf)
                        (imsg--start-local-process buf))))))
      (setq imsg--process proc)
      (setq imsg--partial "")
      (set-process-query-on-exit-flag proc nil)))
  imsg--process)

(defun imsg-stop ()
  "Stop the imsg RPC process."
  (interactive)
  (when (process-live-p imsg--process)
    (delete-process imsg--process))
  (setq imsg--process nil)
  (clrhash imsg--pending)
  (clrhash imsg--subscriptions))

(defun imsg--sentinel (_proc event)
  (unless (string-prefix-p "finished" event)
    (maphash
     (lambda (_id callback)
       (when callback
         (funcall callback nil (list (cons "message" "rpc process exited")))))
     imsg--pending)
    (clrhash imsg--pending)
    (clrhash imsg--subscriptions)
    (clrhash imsg--subscription-tokens)
    (when (and imsg-auto-reconnect (or imsg-remote-directory t))
      (run-at-time
       0.5 nil
       (lambda ()
         (when (not (process-live-p imsg--process))
           (condition-case err
               (progn
                 (imsg--ensure-process)
                 (imsg--resubscribe-all))
             (error (message "imsg: reconnect failed (%s)" err)))))))))

(defun imsg--json-read (payload)
  "Parse JSON string PAYLOAD into an alist."
  (if (fboundp 'json-parse-string)
      (json-parse-string payload
                         :object-type 'alist
                         :array-type 'list
                         :null-object nil
                         :false-object nil)
    (let ((json-object-type 'alist)
          (json-array-type 'list)
          (json-false nil))
      (json-read-from-string payload))))

(defun imsg--subscription-key (value)
  (cond
   ((stringp value) value)
   ((numberp value) (number-to-string value))
   (t (format "%s" value))))

(defun imsg--filter (_proc chunk)
  (setq imsg--partial (concat imsg--partial chunk))
  (while (string-match "\n" imsg--partial)
    (let ((line (substring imsg--partial 0 (match-beginning 0))))
      (setq imsg--partial (substring imsg--partial (match-end 0)))
      (unless (string-empty-p line)
        (condition-case err
            (imsg--dispatch (imsg--json-read line))
          (error
           (message "imsg: failed to parse line: %s (%s)" line err)))))))

(defun imsg--dispatch (payload)
  (let ((id (alist-get 'id payload nil nil #'equal))
        (method (alist-get 'method payload nil nil #'equal)))
    (cond
     (id (imsg--handle-response payload))
     (method (imsg--handle-notification payload))
     (t (message "imsg: unknown payload %S" payload)))))

(defun imsg--handle-response (payload)
  (let* ((id (alist-get 'id payload nil nil #'equal))
         (callback (gethash id imsg--pending))
         (result (alist-get 'result payload nil nil #'equal))
         (error (alist-get 'error payload nil nil #'equal)))
    (remhash id imsg--pending)
    (when callback
      (funcall callback result error))))

(defun imsg--handle-notification (payload)
  (let* ((method (alist-get 'method payload nil nil #'equal))
         (params (alist-get 'params payload nil nil #'equal)))
    (when (string= method "message")
      (let* ((subscription (imsg--subscription-key
                            (alist-get 'subscription params nil nil #'equal)))
             (message (alist-get 'message params nil nil #'equal))
             (callback (gethash subscription imsg--subscriptions)))
        (when callback
          (funcall callback message))
        (when message
          (imsg--cache-contacts (list (alist-get 'sender message)))
          (when imsg-notify-enabled
            (funcall imsg-notify-function message))
          (run-hook-with-args 'imsg-on-message-hook message))))))

(defun imsg--default-notify (message)
  (let* ((sender (imsg--sender-display message))
         (text (or (alist-get 'text message) ""))
         (title (if (string-empty-p sender) "iMessage" sender)))
    (if (fboundp 'notifications-notify)
        (notifications-notify
         :title title
         :body text
         :app-name "imsg")
      (message "imsg: %s %s" title text))))

(defun imsg-contacts-search (query &optional limit)
  "Search contacts by QUERY."
  (let ((params `(("query" . ,query))))
    (when limit (setq params (append params `(("limit" . ,limit)))))
    (imsg-request-sync "contacts.search" params)))

(defun imsg-contacts-resolve (handles)
  "Resolve HANDLE list to contact names."
  (imsg-request-sync "contacts.resolve" `(("handles" . ,handles))))

(defun imsg--cache-contacts (handles)
  (let* ((unknown (cl-remove-if (lambda (h) (gethash h imsg--contact-cache)) handles)))
    (when unknown
      (condition-case _err
          (let* ((result (imsg-contacts-resolve unknown))
                 (contacts (alist-get 'contacts result)))
            (dolist (entry contacts)
              (let ((handle (alist-get 'handle entry))
                    (name (alist-get 'name entry)))
                (when (and handle name)
                  (puthash handle name imsg--contact-cache)))))
        (error nil)))))

(defun imsg--sender-display (message)
  (let* ((sender (or (alist-get 'sender message) "")))
    (or (gethash sender imsg--contact-cache) sender)))

(defvar-local imsg--compose-target nil)

(define-derived-mode imsg-compose-mode text-mode "IMsg-Compose"
  "Major mode for composing imsg messages."
  (setq-local header-line-format "C-c C-c send, C-c C-k cancel"))

(defun imsg-compose-chat (chat-id)
  "Compose a message to CHAT-ID."
  (interactive "nChat ID: ")
  (let ((buf (get-buffer-create "*imsg-compose*")))
    (with-current-buffer buf
      (erase-buffer)
      (imsg-compose-mode)
      (setq imsg--compose-target (list :chat-id chat-id)))
    (pop-to-buffer buf)))

(defun imsg-compose-to (to)
  "Compose a direct message to TO or a contact name."
  (interactive
   (list
    (let* ((query (read-string "To (handle/number or name): "))
           (matches (condition-case _err
                        (alist-get 'matches (imsg-contacts-search query 10))
                      (error nil)))
           (choices (and matches
                         (cl-mapcan
                          (lambda (match)
                            (let ((name (alist-get 'name match))
                                  (handles (alist-get 'handles match)))
                              (mapcar (lambda (handle)
                                        (cons (format "%s <%s>" name handle) handle))
                                      handles)))
                          matches))))
      (if (and choices (not (string-empty-p query)))
          (cdr (assoc (completing-read "Select contact: " choices nil t) choices))
        (completing-read "To (handle/number or name): "
                         (append (mapcar #'car choices) imsg--recipient-history)
                         nil nil query 'imsg--recipient-history)))))
  (let ((buf (get-buffer-create "*imsg-compose*")))
    (with-current-buffer buf
      (erase-buffer)
      (imsg-compose-mode)
      (setq imsg--compose-target (list :to to)))
    (pop-to-buffer buf)))

(defun imsg-compose-send ()
  "Send the current compose buffer."
  (interactive)
  (unless (eq major-mode 'imsg-compose-mode)
    (user-error "Not in an imsg compose buffer"))
  (let* ((text (string-trim (buffer-string)))
         (chat-id (plist-get imsg--compose-target :chat-id))
         (to (plist-get imsg--compose-target :to)))
    (when (string-empty-p text)
      (user-error "Message text is empty"))
    (cond
     (chat-id
      (imsg-send `(("chat_id" . ,chat-id) ("text" . ,text))))
     (to
      (imsg-send `(("to" . ,to) ("text" . ,text))))
     (t
      (user-error "Missing compose target")))
    (when to
      (setq imsg--recipient-history (cons to (delete to imsg--recipient-history))))
    (kill-buffer (current-buffer))
    (message "imsg: sent")))

(defun imsg-compose-last-recipient ()
  "Compose a message to the most recent recipient."
  (interactive)
  (if (car imsg--recipient-history)
      (imsg-compose-to (car imsg--recipient-history))
    (user-error "No recipient history")))

(defun imsg-help ()
  "Show an imsg help overlay."
  (interactive)
  (let ((buf (get-buffer-create "*imsg-help*")))
    (with-current-buffer buf
      (setq buffer-read-only nil)
      (erase-buffer)
      (insert "imsg help\n\n")
      (insert "Commands\n")
      (insert "  imsg-transient: main command menu\n")
      (insert "  imsg-chats-list-interactive\n")
      (insert "  imsg-history-interactive\n")
      (insert "  imsg-watch-subscribe-interactive\n")
      (insert "  imsg-watch-unsubscribe-interactive\n")
      (insert "  imsg-compose-chat / imsg-compose-to\n\n")
      (insert "Compose keys\n")
      (insert "  C-c C-c send\n")
      (insert "  C-c C-k cancel\n")
      (insert "  C-c C-e emoji chooser\n")
      (insert "  C-c C-r reaction\n")
      (insert "  C-c C-t compose menu\n\n")
      (insert "Transient\n")
      (insert "  ? help\n")
      (insert "  L last recipient\n")
      (setq buffer-read-only t)
      (special-mode))
    (display-buffer buf '((display-buffer-full-frame)))))

(defun imsg-compose-cancel ()
  "Cancel the current compose buffer."
  (interactive)
  (when (eq major-mode 'imsg-compose-mode)
    (kill-buffer (current-buffer))
    (message "imsg: cancelled")))

(define-key imsg-compose-mode-map (kbd "C-c C-c") #'imsg-compose-send)
(define-key imsg-compose-mode-map (kbd "C-c C-k") #'imsg-compose-cancel)
(define-key imsg-compose-mode-map (kbd "C-c C-e") #'imsg-compose-insert-emoji)
(define-key imsg-compose-mode-map (kbd "C-c C-r") #'imsg-compose-send-reaction)
(define-key imsg-compose-mode-map (kbd "C-c C-t") #'imsg-compose-menu)

(transient-define-prefix imsg-compose-menu ()
  "Compose menu."
  [["Compose"
    ("s" "send" imsg-compose-send)
    ("e" "emoji" imsg-compose-insert-emoji)
    ("r" "react" imsg-compose-send-reaction)
    ("l" "last recipient" imsg-compose-last-recipient)
    ("?" "help" imsg-help)
    ("c" "cancel" imsg-compose-cancel)]])

(defun imsg-compose-insert-emoji ()
  "Insert an emoji via the built-in chooser."
  (interactive)
  (if (fboundp 'emoji-search)
      (call-interactively 'emoji-search)
    (insert (read-string "Emoji: "))))

(defun imsg--reaction-choice ()
  (completing-read "Reaction: " '("like" "love" "laugh" "emphasis" "question" "dislike") nil nil))

(defun imsg-compose-send-reaction ()
  "Send a reaction to a message GUID."
  (interactive)
  (let ((guid (read-string "Message GUID: "))
        (reaction (imsg--reaction-choice)))
    (imsg-request-sync "reactions.send" `(("guid" . ,guid) ("reaction" . ,reaction)))
    (message "imsg: reaction sent")))

(defun imsg--resubscribe-all ()
  "Resubscribe to all desired subscriptions after reconnect."
  (maphash
   (lambda (token entry)
     (let ((params (plist-get entry :params))
           (callback (plist-get entry :callback)))
       (imsg-watch-subscribe params callback
                             (lambda (subscription _err)
                               (when subscription
                                 (puthash subscription token imsg--subscription-tokens))))))
   imsg--desired-subscriptions))

(defun imsg-request (method params &optional callback)
  "Send a JSON-RPC request to METHOD with PARAMS.
CALLBACK is invoked with (result error) when a response arrives."
  (let* ((proc (imsg--ensure-process))
         (id (format "%d" (cl-incf imsg--next-id)))
         (payload `(("jsonrpc" . "2.0")
                    ("id" . ,id)
                    ("method" . ,method))))
    (when params
      (setq payload (append payload `(("params" . ,params)))))
    (when callback
      (puthash id callback imsg--pending))
    (process-send-string proc (concat (json-encode payload) "\n"))
    id))

(defun imsg-request-sync (method params &optional timeout)
  "Send a JSON-RPC request and block until a response arrives."
  (let* ((proc (imsg--ensure-process))
         (timeout (or timeout imsg-request-timeout))
         (done nil)
         (result nil)
         (err nil))
    (imsg-request
     method params
     (lambda (res error)
       (setq result res)
       (setq err error)
       (setq done t)))
    (while (and (not done)
                (accept-process-output proc timeout)))
    (unless done
      (error "imsg: request timed out"))
    (when err
      (error "imsg: %S" err))
    result))

(defun imsg-chats-list (&optional limit callback)
  "List recent chats.
If CALLBACK is provided, invoke it with the result asynchronously."
  (let ((params (when limit `(("limit" . ,limit)))))
    (if callback
        (imsg-request "chats.list" params callback)
      (imsg-request-sync "chats.list" params))))

(defun imsg-messages-history (chat-id &optional limit participants start end attachments callback)
  "Fetch recent messages for CHAT-ID."
  (let ((params `(("chat_id" . ,chat-id))))
    (when limit (setq params (append params `(("limit" . ,limit)))))
    (when participants (setq params (append params `(("participants" . ,participants)))))
    (when start (setq params (append params `(("start" . ,start)))))
    (when end (setq params (append params `(("end" . ,end)))))
    (when attachments (setq params (append params `(("attachments" . t)))))
    (if callback
        (imsg-request "messages.history" params callback)
      (imsg-request-sync "messages.history" params))))

(defun imsg-send (params &optional callback)
  "Send a message using PARAMS alist.
Example: (imsg-send '((\"to\" . \"+15551234567\") (\"text\" . \"hi\")))."
  (if callback
      (imsg-request "send" params callback)
    (imsg-request-sync "send" params)))

(defun imsg-watch-subscribe (params message-callback &optional ready-callback)
  "Subscribe to message updates.
PARAMS is an alist of RPC parameters. MESSAGE-CALLBACK is invoked per message.
READY-CALLBACK is invoked with (subscription-id error) after subscribing."
  (let ((token (format "sub-%d" (cl-incf imsg--next-id))))
    (puthash token (list :params params :callback message-callback) imsg--desired-subscriptions)
    (imsg-request
     "watch.subscribe" params
     (lambda (result err)
       (if err
           (when ready-callback
             (funcall ready-callback nil err))
         (let ((subscription (imsg--subscription-key
                              (alist-get 'subscription result nil nil #'equal))))
           (when message-callback
             (puthash subscription message-callback imsg--subscriptions)
             (puthash subscription token imsg--subscription-tokens))
           (when ready-callback
             (funcall ready-callback subscription nil))))))))

(defun imsg-watch-unsubscribe (subscription &optional callback)
  "Unsubscribe from message updates."
  (let* ((key (imsg--subscription-key subscription))
         (token (gethash key imsg--subscription-tokens))
         (params `(("subscription" . ,subscription))))
    (imsg-request
     "watch.unsubscribe" params
     (lambda (result err)
       (unless err
         (remhash key imsg--subscriptions))
       (when token
         (remhash token imsg--desired-subscriptions)
         (remhash key imsg--subscription-tokens))
       (when callback
         (funcall callback result err))))))

(defun imsg-running-p ()
  "Return non-nil if the RPC process is running."
  (process-live-p imsg--process))

(defun imsg--tramp-directory (&optional host user method)
  (let ((host (or host imsg-remote-host))
        (user (or user imsg-remote-user))
        (method (or method imsg-remote-method)))
    (if (and user (not (string-empty-p user)))
        (format "/%s:%s@%s:" method user host)
      (format "/%s:%s:" method host))))

(defun imsg-use-remote (&optional host user method)
  "Configure TRAMP to run `imsg rpc` on HOST.
USER and METHOD are optional. This sets `imsg-remote-directory`."
  (interactive "sHost: \nsUser (optional): ")
  (setq imsg-transport 'tramp)
  (setq imsg-remote-directory (imsg--tramp-directory host user method)))

(defun imsg-use-local ()
  "Disable TRAMP and run `imsg rpc` locally."
  (interactive)
  (setq imsg-transport 'local)
  (setq imsg-remote-directory nil))

(defun imsg-use-network (host port)
  "Use a TCP transport to an `imsg rpc` socket."
  (interactive "sHost: \nnPort: ")
  (setq imsg-transport 'network)
  (setq imsg-network-host host)
  (setq imsg-network-port port))

(defun imsg-status ()
  "Show current connection status."
  (interactive)
  (message "imsg: %s (%s)"
           (if (imsg-running-p) "running" "stopped")
           (or imsg-remote-directory "local")))

(defun imsg--format-chat (chat)
  (let* ((chat-name (or (alist-get 'name chat) ""))
         (identifier (or (alist-get 'identifier chat) ""))
         (contact-name (and (not (string-empty-p identifier))
                            (gethash identifier imsg--contact-cache)))
         (label (cond
                 ((string-empty-p chat-name)
                  (if (and contact-name (not (string-empty-p contact-name)))
                      (format "%s (%s)" contact-name identifier)
                    identifier))
                 ((string-empty-p identifier)
                  chat-name)
                 ((or (not contact-name) (string-empty-p contact-name)
                      (string= contact-name chat-name))
                  (format "%s (%s)" chat-name identifier))
                 (t
                  (format "%s (%s, %s)" chat-name contact-name identifier)))))
    (format "[%s] %s last=%s"
            (alist-get 'id chat)
            label
            (or (alist-get 'last_message_at chat) ""))))

(defun imsg--format-message (message)
  (let* ((is-from-me (alist-get 'is_from_me message))
         (direction (if is-from-me "sent" "recv"))
         (sender (imsg--sender-display message))
         (text (or (alist-get 'text message) ""))
         (timestamp (or (alist-get 'created_at message) ""))
         (reply (alist-get 'reply_to_guid message))
         (reactions (alist-get 'reactions message))
         (face (if is-from-me 'imsg-sent-face 'imsg-recv-face))
         (reply-line (when reply
                       (propertize (format "reply to %s" reply) 'face 'imsg-reply-face)))
         (reaction-line (when reactions
                          (let ((summary (imsg--reaction-summary reactions)))
                            (when summary
                              (propertize (format "reactions: %s" summary) 'face 'imsg-reply-face))))))
    (string-join
     (delq nil
           (list
            (propertize (format "%s [%s] %s:" timestamp direction sender) 'face face)
            reply-line
            (propertize text 'face face)
            reaction-line))
     "\n")))

(defun imsg--reaction-summary (reactions)
  "Build a compact reaction summary from REACTIONS."
  (let ((counts (make-hash-table :test 'equal)))
    (dolist (reaction reactions)
      (let ((emoji (or (alist-get 'emoji reaction) "")))
        (unless (string-empty-p emoji)
          (puthash emoji (1+ (gethash emoji counts 0)) counts))))
    (let (parts)
      (maphash (lambda (emoji count)
                 (push (if (> count 1)
                           (format "%s %d" emoji count)
                         emoji)
                       parts))
               counts)
      (when parts
        (string-join (sort parts #'string<) " ")))))

(defun imsg--display-lines (buffer-name lines)
  (let ((buf (get-buffer-create buffer-name)))
    (with-current-buffer buf
      (setq buffer-read-only nil)
      (erase-buffer)
      (dolist (line lines)
        (insert line "\n"))
      (goto-address-mode 1)
      (setq buffer-read-only t))
    (display-buffer buf)))

(defun imsg-chats-list-interactive (limit)
  "Interactive chat list."
  (interactive "nLimit: ")
  (let* ((result (imsg-chats-list limit))
         (chats (alist-get 'chats result)))
    (imsg--cache-contacts
     (delete-dups
      (delq nil (mapcar (lambda (chat) (alist-get 'identifier chat)) chats))))
    (imsg--display-lines "*imsg-chats*"
                         (mapcar #'imsg--format-chat chats))))

(defun imsg-history-interactive (chat-id limit)
  "Interactive history viewer."
  (interactive "nChat ID: \nnLimit: ")
  (let* ((result (imsg-messages-history chat-id limit))
         (messages (alist-get 'messages result)))
    (imsg--cache-contacts (delete-dups (mapcar (lambda (m) (alist-get 'sender m)) messages)))
    (imsg--display-lines "*imsg-history*"
                         (mapcar #'imsg--format-message messages))))

(defun imsg-send-interactive (to text &optional file service)
  "Interactive send."
  (interactive "sTo (handle/number): \nsText: \nsFile (optional): \nsService (auto/imessage/sms): ")
  (let ((params `(("to" . ,to))))
    (when (not (string-empty-p text))
      (setq params (append params `(("text" . ,text)))))
    (when (and file (not (string-empty-p file)))
      (setq params (append params `(("file" . ,file)))))
    (when (and service (not (string-empty-p service)))
      (setq params (append params `(("service" . ,service)))))
    (imsg-send params)
    (message "imsg: send ok")))

(defvar imsg--watch-buffer "*imsg-watch*")

(defun imsg-watch-subscribe-interactive (chat-id)
  "Interactive watch subscribe."
  (interactive "nChat ID (0 for all): ")
  (let ((params (if (> chat-id 0) `(("chat_id" . ,chat-id)) nil)))
    (imsg-watch-subscribe
     params
     (lambda (message)
       (imsg--cache-contacts (list (alist-get 'sender message)))
       (with-current-buffer (get-buffer-create imsg--watch-buffer)
         (goto-address-mode 1)
         (goto-char (point-max))
         (insert (imsg--format-message message) "\n")))
     (lambda (subscription err)
       (if err
           (message "imsg: watch error %S" err)
         (message "imsg: watch subscribed %s" subscription)
         (display-buffer imsg--watch-buffer))))))

(defun imsg-watch-unsubscribe-interactive (subscription)
  "Interactive watch unsubscribe."
  (interactive "sSubscription ID: ")
  (imsg-watch-unsubscribe
   subscription
   (lambda (_result err)
     (if err
         (message "imsg: unsubscribe error %S" err)
       (message "imsg: unsubscribed %s" subscription)))))

(transient-define-prefix imsg-transient ()
  "imsg command menu."
  [["Connect"
    ("r" "use remote" imsg-use-remote)
    ("l" "use local" imsg-use-local)
    ("n" "use network" imsg-use-network)
    ("s" "status" imsg-status)
    ("k" "stop" imsg-stop)]
   ["Chats/Messages"
    ("c" "list chats" imsg-chats-list-interactive)
    ("h" "history" imsg-history-interactive)]
   ["Send/Watch"
    ("m" "send" imsg-send-interactive)
    ("C" "compose chat" imsg-compose-chat)
    ("D" "compose direct" imsg-compose-to)
    ("L" "last recipient" imsg-compose-last-recipient)
    ("w" "watch subscribe" imsg-watch-subscribe-interactive)
    ("u" "watch unsubscribe" imsg-watch-unsubscribe-interactive)]
   ["Help"
    ("?" "help" imsg-help)]])

(provide 'imsg)
;;; imsg.el ends here
