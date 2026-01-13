;;; imsg-test.el --- Tests for imsg.el -*- lexical-binding: t; -*-

(require 'ert)
(require 'cl-lib)
(require 'imsg)

(ert-deftest imsg-tramp-directory-defaults ()
  (let ((imsg-remote-host "192.168.2.186")
        (imsg-remote-user nil)
        (imsg-remote-method "ssh"))
    (should (string= (imsg--tramp-directory) "/ssh:192.168.2.186:"))))

(ert-deftest imsg-tramp-directory-with-user ()
  (let ((imsg-remote-host "192.168.2.186")
        (imsg-remote-user "jonathan")
        (imsg-remote-method "ssh"))
    (should (string= (imsg--tramp-directory) "/ssh:jonathan@192.168.2.186:"))))

(ert-deftest imsg-use-remote-sets-directory ()
  (let ((imsg-remote-host "192.168.2.186")
        (imsg-remote-user nil)
        (imsg-remote-method "ssh")
        (imsg-remote-directory nil))
    (imsg-use-remote)
    (should (string= imsg-remote-directory "/ssh:192.168.2.186:"))))

(ert-deftest imsg-ensure-process-selects-remote ()
  (let ((imsg-remote-directory "/ssh:192.168.2.186:")
        (imsg--process nil)
        (remote-started nil)
        (local-started nil))
    (cl-letf (((symbol-function 'imsg--start-remote-process)
               (lambda (_buf)
                 (setq remote-started t)
                 (start-process "imsg-test-remote" nil "cat")))
              ((symbol-function 'imsg--start-local-process)
               (lambda (_buf)
                 (setq local-started t)
                 (start-process "imsg-test-local" nil "cat"))))
      (unwind-protect
          (progn
            (imsg--ensure-process)
            (should remote-started)
            (should (not local-started)))
        (imsg-stop)))))

(ert-deftest imsg-start-remote-process-uses-tramp-directory ()
  (let ((imsg-remote-directory "/ssh:jonathan@192.168.2.186:")
        (imsg-binary "imsg")
        (imsg-db-path nil)
        (imsg-rpc-extra-args nil)
        (captured-dir nil)
        (captured-args nil))
    (cl-letf (((symbol-function 'start-file-process)
               (lambda (_name _buffer &rest args)
                 (setq captured-dir default-directory)
                 (setq captured-args args)
                 (start-process "imsg-test-remote" nil "cat"))))
      (unwind-protect
          (progn
            (imsg--start-remote-process (get-buffer-create "*imsg-test*"))
            (should (string= captured-dir "/ssh:jonathan@192.168.2.186:"))
            (should (equal captured-args '("imsg" "rpc"))))
        (imsg-stop)))))

(ert-deftest imsg-use-remote-with-custom-method ()
  (let ((imsg-remote-host "192.168.2.186")
        (imsg-remote-user "jonathan")
        (imsg-remote-method "sshx"))
    (should (string= (imsg--tramp-directory) "/sshx:jonathan@192.168.2.186:"))))

(ert-deftest imsg-network-transport-selects-network-process ()
  (let ((imsg-transport 'network)
        (imsg--process nil)
        (network-started nil))
    (cl-letf (((symbol-function 'imsg--start-network-process)
               (lambda (_buf)
                 (setq network-started t)
                 (start-process "imsg-test-network" nil "cat"))))
      (unwind-protect
          (progn
            (imsg--ensure-process)
            (should network-started))
        (imsg-stop)))))

(ert-deftest imsg-notify-calls-custom-function ()
  (let ((imsg-notify-enabled t)
        (imsg-notify-function nil)
        (called nil))
    (setq imsg-notify-function
          (lambda (_message) (setq called t)))
    (imsg--handle-notification
     '((method . "message")
       (params . ((subscription . 1)
                  (message . ((sender . "Test") (text . "Hi")))))))
    (should called)))

(ert-deftest imsg-remote-ssh-auth-works ()
  (let ((enabled (getenv "IMSG_REMOTE_E2E"))
        (host (or (getenv "IMSG_REMOTE_HOST") "192.168.2.186"))
        (user (or (getenv "IMSG_REMOTE_USER") "jonathan")))
    (unless enabled
      (ert-skip "IMSG_REMOTE_E2E not set"))
    (let ((exit-code
           (process-file
            "ssh" nil nil nil
            "-o" "BatchMode=yes"
            "-o" "ConnectTimeout=5"
            (format "%s@%s" user host)
            "true")))
      (should (eq exit-code 0)))))

(ert-deftest imsg-resubscribe-all-replays ()
  (let ((imsg--desired-subscriptions (make-hash-table :test 'equal))
        (called nil))
    (puthash "token-1" (list :params '(("chat_id" . 1)) :callback #'ignore)
             imsg--desired-subscriptions)
    (cl-letf (((symbol-function 'imsg-watch-subscribe)
               (lambda (params _callback _ready)
                 (setq called params))))
      (imsg--resubscribe-all)
      (should (equal called '(("chat_id" . 1)))))))

(ert-deftest imsg-image-rendering-team-wilner ()
  (let ((enabled (getenv "IMSG_IMAGE_TEST")))
    (unless enabled
      (ert-skip "IMSG_IMAGE_TEST not set"))
    (let* ((host (or (getenv "IMSG_RPC_HOST") "192.168.2.186"))
           (port (string-to-number (or (getenv "IMSG_RPC_PORT") "57999")))
           (imsg-transport 'network)
           (imsg-network-host host)
           (imsg-network-port port)
           (imsg-request-timeout 10)
           (imsg-history-timeout 20))
      (unwind-protect
          (let* ((result (imsg-chats-list 100))
                 (chats (alist-get 'chats result))
                 (chat (cl-find-if (lambda (entry)
                                     (string= (alist-get 'name entry) "Team Wilner"))
                                   chats)))
            (unless chat
              (ert-skip "Team Wilner chat not found"))
            (let* ((chat-id (alist-get 'id chat))
                   (history (imsg-messages-history-sync chat-id 20 nil nil nil t imsg-history-timeout))
                   (messages (alist-get 'messages history))
                   (paths nil))
              (dolist (message messages)
                (let ((attachments (alist-get 'attachments message)))
                  (when (listp attachments)
                    (dolist (attachment attachments)
                      (let ((path (imsg--attachment-path attachment)))
                        (when (and path (imsg--attachment-image-p attachment))
                          (push path paths)))))))
              (unless paths
                (ert-fail "No image attachments found in last 20 messages"))
              (cl-letf (((symbol-function 'display-images-p) (lambda (&rest _) t)))
                (let ((img (imsg--image-from-path (car paths))))
                  (should img)))))
        (imsg-stop)))))

(ert-deftest imsg-cache-attachment-writes-file ()
  (let ((tmp (make-temp-file "imsg-cache-" t))
        (imsg-fetch-attachments t))
    (let ((imsg-attachment-cache-dir tmp))
      (cl-letf (((symbol-function 'imsg-request-sync)
                 (lambda (_method _params _timeout)
                   (list (cons 'data "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMB/7l1i5QAAAAASUVORK5CYII=")
                         (cons 'filename "test.png")))))
        (let ((path (imsg--cache-attachment "/Users/jonathan/Pictures/test.heic")))
          (should (and path (file-exists-p path))))))))

(provide 'imsg-test)
;;; imsg-test.el ends here
